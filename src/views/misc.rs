use crate::sql::*;
use crate::utils::*;
use askama::Template;
use axum::body::{boxed, Full};
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use itertools::Itertools;
use mime_guess::mime;
use serde::Serialize;
use sqlx::{query_as, FromRow};
use std::collections::{HashMap, HashSet};

typed_path!("/static/*path", StaticFiles, path);
pub async fn static_files(StaticFiles { path }: StaticFiles) -> Result<impl IntoResponse> {
    #[derive(rust_embed::RustEmbed)]
    #[folder = "static"]
    struct Asset;

    match Asset::get(path.as_str().trim_start_matches('/')) {
        Some(content) => {
            let body = boxed(Full::from(content.data));
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Ok(Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(body)?)
        }
        None => not_found!("/static{path}"),
    }
}

typed_path!("/pkgtrie.js", PkgTrie);
pub async fn pkgtrie(_: PkgTrie, db: Ext) -> Result<impl IntoResponse> {
    #[derive(Default, Clone, Debug)]
    struct Trie {
        children: HashMap<char, Self>,
        is_end: bool,
    }

    #[derive(Debug, FromRow)]
    struct Package {
        name: String,
    }

    impl Trie {
        fn insert(&mut self, word: &str) {
            let mut cur = self;
            for c in word.chars() {
                cur = cur.children.entry(c).or_insert_with(Default::default);
            }
            cur.is_end = true;
        }
        fn walk_tree(&self) -> String {
            let mut res = vec![];
            for (char, trie) in &self.children {
                res.push(format!("'{char}':{}", trie.walk_tree()));
            }
            if self.is_end {
                res.push("$:0".into());
            }
            format!("{{{}}}", res.join(","))
        }
    }

    let pkgs: Vec<Package> = query_as("SELECT name FROM packages").fetch_all(&db.abbs).await?;

    let mut trie: Trie = Default::default();
    pkgs.iter().for_each(|pkg| trie.insert(&pkg.name));
    let packagetrie = trie.walk_tree().replace("{$:0}", "0");

    build_resp(
        mime::APPLICATION_JAVASCRIPT.as_ref(),
        format!("var pkgTrie = {packagetrie};"),
    )
}

typed_path!("/list.json", PkgList);
pub async fn pkglist(_: PkgList, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        branch: String,
        category: String,
        commit_time: i64,
        committer: String,
        description: String,
        directory: String,
        dpkg_availrepos: String,
        dpkg_version: String,
        full_version: String,
        name: String,
        pkg_section: String,
        section: String,
        tree: String,
        tree_category: String,
        ver_compare: i32,
        version: String,
    }

    #[derive(Serialize)]
    struct PkgList {
        last_modified: i64,
        packages: Vec<Package>,
    }

    let packages: Vec<Package> = query_as(SQL_GET_PACKAGE_LIST).fetch_all(&db.abbs).await?;

    let res = PkgList {
        last_modified: db_last_modified(db).await?,
        packages,
    };

    let json = serde_json::to_string(&res)?;

    Ok(build_resp(mime::APPLICATION_JSON.as_ref(), json))
}

typed_path!("/cleanmirror/*repo", CleanMirror, repo);
pub async fn cleanmirror(CleanMirror { repo }: CleanMirror, q: Query, db: Ext) -> Result<impl IntoResponse> {
    let reason: Option<HashSet<_>> = q
        .get_reason()
        .as_ref()
        .map(|r| r.split(',').map(|x| x.to_string()).collect());
    let repo = strip_prefix(&repo);

    #[derive(Debug, FromRow, Serialize)]
    struct Deb {
        filename: String,
        removereason: String,
    }

    #[derive(Debug, Template, Serialize)]
    #[template(path = "cleanmirror.txt")]
    struct Template<'a> {
        debs: Vec<&'a Deb>,
    }

    let mut debs: Vec<Deb> = if get_repo(repo, &db).await?.realname == "noarch" {
        query_as(SQL_GET_DEB_LIST_NOARCH)
    } else {
        query_as(SQL_GET_DEB_LIST_HASARCH)
    }
    .bind(repo)
    .bind(repo)
    .bind(repo)
    .fetch_all(&db.abbs)
    .await?;

    let debs = if let Some(reason) = reason {
        debs.iter_mut()
            .filter_map(|deb| {
                let v = deb
                    .removereason
                    .split(',')
                    .filter(|r| reason.contains(*r))
                    .collect_vec();
                if v.is_empty() {
                    None
                } else {
                    let reason = v.join(",");
                    if reason != deb.removereason {
                        deb.removereason = reason;
                    }

                    Some(&*deb)
                }
            })
            .collect_vec()
    } else {
        debs.iter().collect_vec()
    };

    let ctx = Template { debs };

    render::<_, Template>(ctx, None, &q)
}
