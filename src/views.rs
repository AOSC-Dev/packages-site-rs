use crate::db::{get_page, Page, PAGESIZE};
use crate::utils::*;
use abbs_meta_tree::package::FailArch;
use anyhow::{anyhow, Context};
use askama::Template;
use askama_axum::Response;
use axum::body::{boxed, Full};
use axum::headers::{ETag, IfNoneMatch};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect};
use axum::TypedHeader;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use mime_guess::mime;
use serde::Serialize;
use sqlx::{query, query_as, FromRow};
use std::collections::{HashMap, HashSet};
use std::iter::repeat;

pub async fn fallback(uri: Uri) -> impl IntoResponse {
    Error::NotFound(format!("No route for {}", uri))
}

typed_path!("/static/*path", StaticFiles, path);
pub async fn static_files(
    StaticFiles { path }: StaticFiles,
    if_none_match: Option<TypedHeader<IfNoneMatch>>,
) -> Result<impl IntoResponse> {
    #[derive(rust_embed::RustEmbed)]
    #[folder = "static"]
    struct Asset;

    match Asset::get(path.as_str().trim_start_matches('/')) {
        Some(content) => {
            let hash = hex::encode(content.metadata.sha256_hash());
            let etag = format!(r#"{:?}"#, hash)
                .parse::<ETag>()
                .with_context(|| "failed to convert hash to etag")?;
            if let Some(if_none_match) = if_none_match {
                if if_none_match.precondition_passes(&etag) {
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            let body = boxed(Full::from(content.data));
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Ok(Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::ETAG, format!("\"{hash}\""))
                .body(body)?)
        }
        None => not_found!("/static{path}"),
    }
}

typed_path!("/changelog/:name", Changelog, name);
pub async fn changelog(
    Changelog { name }: Changelog,
    q: Query,
    db: Ext,
) -> Result<impl IntoResponse> {
    #[derive(Debug, FromRow, Serialize)]
    struct Change {
        pub package: String,
        pub githash: String,
        pub version: String,
        pub branch: String,
        pub urgency: String,
        pub message: String,
        pub maintainer_name: String,
        pub maintainer_email: String,
        pub timestamp: i64,
    }

    let changes: Vec<Change> =
        query_as("SELECT * FROM package_changes WHERE package = ? ORDER BY timestamp DESC")
            .bind(&name)
            .fetch_all(&db.abbs)
            .await?;

    if changes.is_empty() {
        return not_found!("Package \"{name}\" not found.");
    }

    #[derive(Template, Serialize)]
    #[template(path = "changelog.txt")]
    struct Template {
        changes: Vec<Change>,
    }

    let ctx = Template { changes };

    render::<_, Template>(ctx, None, &q)
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

    let pkgs: Vec<Package> = query_as("SELECT name FROM packages")
        .fetch_all(&db.abbs)
        .await?;

    let mut trie: Trie = Default::default();
    pkgs.iter().for_each(|pkg| trie.insert(&pkg.name));
    let packagetrie = trie.walk_tree().replace("{$:0}", "0");

    build_resp(
        mime::JAVASCRIPT.as_ref(),
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

    Ok(build_resp(mime::JSON.as_ref(), json))
}

typed_path!("/lagging/*repo", Lagging, repo);
pub async fn lagging(Lagging { repo }: Lagging, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Debug, Serialize)]
    struct Package {
        name: String,
        dpkg_version: String,
        full_version: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "lagging.html")]
    struct Template<'a> {
        page: Page,
        repo: String,
        packages: &'a Vec<Package>,
    }

    #[derive(Template)]
    #[template(path = "lagging.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<Package>,
    }

    let repo = strip_prefix(&repo)?;
    let architecture = get_repo(repo, &db).await?.architecture;

    let (page, ref packages) = get_page!(
        SQL_GET_PACKAGE_LAGGING,
        Package,
        q.get_page(),
        &db.abbs,
        repo,
        architecture
    )
    .await?;

    if packages.is_empty() {
        return not_found!("There's no lagging packages.");
    }

    let ctx = Template {
        page,
        repo: repo.to_string(),
        packages,
    };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/missing/*repo", Missing, repo);
pub async fn missing(Missing { repo }: Missing, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Debug, Serialize)]
    struct Package {
        name: String,
        description: String,
        full_version: String,
        dpkg_version: String,
        tree_category: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "missing.html")]
    struct Template<'a> {
        page: Page,
        repo: String,
        packages: &'a Vec<Package>,
    }

    #[derive(Template)]
    #[template(path = "missing.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<Package>,
    }

    let repo = strip_prefix(&repo)?;
    let repo = get_repo(repo, &db).await?;

    let (page, ref packages) = get_page!(
        SQL_GET_PACKAGE_MISSING,
        Package,
        q.get_page(),
        &db.abbs,
        &repo.realname,
        &repo.architecture,
        &repo.realname
    )
    .await?;

    if packages.is_empty() {
        return not_found!("There's no missing packages.");
    }

    let ctx = Template {
        page,
        repo: repo.name.to_string(),
        packages,
    };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/tree/:tree", RouteTree, tree);
pub async fn tree(RouteTree { tree }: RouteTree, q: Query, db: Ext) -> Result<impl IntoResponse> {
    get_tree(&tree, &db).await?;

    #[derive(FromRow, Debug)]
    struct Package {
        name: String,
        dpkg_version: String,
        dpkg_availrepos: String,
        description: String,
        full_version: String,
        ver_compare: i64,
    }

    #[derive(Debug, Serialize)]
    struct TemplatePackage {
        name: String,
        dpkg_version: String,
        dpkg_repos: String,
        description: String,
        full_version: String,
        ver_compare: i64,
    }

    #[derive(Template, Serialize)]
    #[template(path = "tree.html")]
    struct Template<'a> {
        page: Page,
        tree: String,
        packages: &'a Vec<TemplatePackage>,
    }

    #[derive(Template)]
    #[template(path = "tree.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<TemplatePackage>,
    }

    let (page, packages) =
        get_page!(SQL_GET_PACKAGE_TREE, Package, q.get_page(), &db.abbs, &tree).await?;

    if packages.is_empty() {
        return not_found!("There's no packages.");
    }

    let packages = &packages
        .into_iter()
        .map(|package| {
            let repos = package.dpkg_availrepos.split(',').sorted().collect_vec();
            let dpkg_repos = repos.join(", ");

            TemplatePackage {
                name: package.name,
                dpkg_version: package.dpkg_version,
                dpkg_repos,
                description: package.description,
                full_version: package.full_version,
                ver_compare: package.ver_compare,
            }
        })
        .collect();

    let ctx = Template {
        page,
        tree,
        packages,
    };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/ghost/*repo", Ghost, repo);
pub async fn ghost(Ghost { repo }: Ghost, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(Debug, FromRow, Serialize)]
    struct Package {
        name: String,
        dpkg_version: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "ghost.html")]
    struct Template<'a> {
        packages: &'a Vec<Package>,
        repo: String,
        page: Page,
    }

    #[derive(Template, Serialize)]
    #[template(path = "ghost.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<Package>,
    }

    let repo = strip_prefix(&repo)?;
    get_repo(repo, &db).await?;

    let (page, ref packages) = get_page!(
        SQL_GET_PACKAGE_GHOST,
        Package,
        q.get_page(),
        &db.abbs,
        &repo
    )
    .await?;

    if packages.is_empty() {
        return not_found!("There's no ghost packages.");
    }

    let ctx = Template {
        packages,
        repo: repo.to_string(),
        page,
    };
    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/search", Search);
pub async fn search(_: Search, query: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow)]
    struct Package {
        full_version: String,
        desc_highlight: String,
        description: String,
        name: String,
    }

    #[derive(FromRow, Serialize)]
    struct PackageTemplate {
        name_highlight: String,
        full_version: String,
        desc_highlight: String,
        description: String,
        name: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "search.html")]
    struct Template<'a> {
        q: &'a String,
        packages: &'a Vec<PackageTemplate>,
        page: Page,
    }

    #[derive(Template)]
    #[template(path = "search.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<PackageTemplate>,
    }

    let q = if let Some(q) = query.get_query() {
        q
    } else {
        let ctx = Template {
            q: &"".to_string(),
            packages: &vec![],
            page: Page::default(),
        };
        let ctx_tsv = TemplateTsv { packages: &vec![] };

        return render(ctx, Some(ctx_tsv), &query);
    };

    if !query.get_noredir() {
        let q = q.trim().to_lowercase().replace(' ', "-").replace('_', "-");
        let mut row = sqlx::query("SELECT 1 FROM packages WHERE name = ?")
            .bind(&&q)
            .fetch_optional(&db.abbs)
            .await?;

        if row.is_none() {
            row = sqlx::query(SQL_GET_PACKAGE_INFO_GHOST)
                .bind(&&q)
                .fetch_optional(&db.abbs)
                .await?;
        }
        if row.is_some() {
            return Ok(Redirect::to(&format!("/packages/{q}")).into_response());
        }
    }

    let qesc = &q;
    //let qesc = RE_FTS5_COLSPEC.replace(&q, r#""\1""#); WIP

    let (page, packages) = get_page!(
        SQL_SEARCH_PACKAGES_DESC,
        Package,
        query.get_page(),
        &db.abbs,
        &qesc,
        &qesc,
        &qesc,
        &qesc,
        &qesc,
        &qesc
    )
    .await?;

    let packages = &packages
        .into_iter()
        .map(|pkg| {
            let desc_highlight = html_escape::encode_safe(&pkg.desc_highlight)
                .replace("&lt;b&gt;", "<b>")
                .replace("&lt;&#x2F;b&gt;", "</b>")
                .replace("%^&amp;", "");

            let name_highlight =
                html_escape::encode_safe(&pkg.name).replace(q, &format!("<b>{q}</b>"));

            PackageTemplate {
                name_highlight,
                full_version: pkg.full_version,
                desc_highlight,
                description: pkg.description,
                name: pkg.name,
            }
        })
        .collect();

    let ctx = Template { q, packages, page };
    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &query)
}

typed_path!("/srcupd/:tree", Srcupd, tree);
pub async fn srcupd(Srcupd { tree }: Srcupd, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        name: String,
        version: String,
        upstream_version: String,
        updated: i64,
        upstream_url: String,
        upstream_tarball: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "srcupd.html")]
    struct Template<'a> {
        page: Page,
        packages: &'a Vec<Package>,
        tree: String,
        section: String,
    }

    #[derive(Template)]
    #[template(path = "srcupd.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<Package>,
    }

    get_tree(&tree, &db).await?;
    let section = q.get_section().clone().unwrap_or_default();

    let (page, ref packages) = get_page!(
        SQL_GET_PACKAGE_SRCUPD,
        Package,
        q.get_page(),
        &db.abbs,
        &tree,
        &section,
        &section
    )
    .await?;

    if packages.is_empty() {
        return not_found!("There's no outdated packages.");
    }

    let ctx = Template {
        page,
        packages,
        tree,
        section,
    };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/updates", Updates);
pub async fn updates(_: Updates, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        name: String,
        dpkg_version: String,
        description: String,
        full_version: String,
        commit_time: i64,
        ver_compare: i64,
    }

    #[derive(Template, Serialize)]
    #[template(path = "updates.html")]
    struct Template<'a> {
        packages: &'a Vec<Package>,
    }

    #[derive(Template)]
    #[template(path = "updates.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<Package>,
    }

    let packages: &Vec<Package> = &query_as(SQL_GET_PACKAGE_NEW_LIST)
        .bind(100)
        .fetch_all(&db.abbs)
        .await?;

    if packages.is_empty() {
        return not_found!("There's no updates.");
    }

    let ctx = Template { packages };
    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/", Index);
pub async fn index(_: Index, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        name: String,
        description: String,
        full_version: String,
        commit_time: i64,
    }

    #[derive(Template, Serialize)]
    #[template(path = "index.html")]
    struct Template {
        total: i64,
        repo_categories: Vec<(String, Vec<Repo>)>,
        source_trees: IndexMap<String, Tree>,
        updates: Vec<Package>,
    }

    let source_trees = db_trees(&db).await?;
    let repos = db_repos(&db).await?;

    let repo_categories = REPO_CAT
        .iter()
        .map(|(category_capital, category)| {
            let repos = repos
                .iter()
                .filter_map(|(_name, repo)| {
                    if &repo.category == category_capital {
                        Some(repo.clone())
                    } else {
                        None
                    }
                })
                .collect();

            (category.to_string(), repos)
        })
        .collect_vec();

    let total: i64 = source_trees.iter().map(|(_name, repo)| repo.pkgcount).sum();
    let updates = query_as(SQL_GET_PACKAGE_NEW).fetch_all(&db.abbs).await?;

    let ctx = Template {
        total,
        repo_categories,
        source_trees,
        updates,
    };

    render::<_, Template>(ctx, None, &q)
}

typed_path!("/repo/*repo", RouteRepo, repo);
pub async fn repo(RouteRepo { repo }: RouteRepo, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow)]
    struct Package {
        name: String,
        full_version: String,
        dpkg_version: String,
        description: String,
    }

    #[derive(Serialize)]
    struct PackageTemplate {
        ver_compare: i64,
        name: String,
        dpkg_version: String,
        description: String,
    }

    #[derive(Template, Serialize)]
    #[template(path = "repo.html")]
    struct Template<'a> {
        packages: &'a Vec<PackageTemplate>,
        repo: String,
        page: Page,
    }

    #[derive(Template, Serialize)]
    #[template(path = "repo.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a Vec<PackageTemplate>,
    }

    let repo = strip_prefix(&repo)?;
    get_repo(repo, &db).await?;

    let (page, packages) =
        get_page!(SQL_GET_PACKAGE_REPO, Package, q.get_page(), &db.abbs, repo).await?;

    let packages = &packages
        .into_iter()
        .map(|pkg| {
            let (latest, fullver) = (pkg.dpkg_version, pkg.full_version);

            let ver_compare = if !latest.is_empty() {
                match deb_version::compare_versions(&latest, &fullver) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                }
            } else {
                -1
            };

            PackageTemplate {
                ver_compare,
                name: pkg.name,
                dpkg_version: latest,
                description: pkg.description,
            }
        })
        .collect_vec();

    let ctx = Template {
        packages,
        repo: repo.into(),
        page,
    };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/packages/:name", RoutePackage, name);
pub async fn packages(
    RoutePackage { name }: RoutePackage,
    q: Query,
    db: Ext,
) -> Result<impl IntoResponse> {
    #[derive(FromRow, Debug, Serialize)]
    struct Package {
        name: String,
        tree: String,
        tree_category: String,
        branch: String,
        category: String,
        section: String,
        pkg_section: String,
        directory: String,
        description: String,
        version: String,
        full_version: String,
        commit_time: i64,
        committer: String,
        dependency: String,
        noarch: bool,
        fail_arch: String,
        srctype: String,
        srcurl: String,
        hasrevdep: bool,
    }

    #[derive(Debug, Serialize)]
    struct MatrixRow {
        repo: String,
        meta: Vec<DpkgMeta>,
    }

    #[derive(Default, Clone, Debug, Serialize)]
    struct DpkgMeta {
        hasmeta: bool,
        version: String,
        testing: i64,
        repo: String,
        size: i64,
    }

    #[derive(FromRow, Debug)]
    struct DpkgPackage {
        version: String,
        //architecture: String,
        repo: String,
        reponame: String,
        testing: i64,
        //filename: String,
        size: i64,
    }

    #[derive(Debug, Serialize)]
    struct Version {
        version: String,
        url: String,
        branch: String,
    }

    #[derive(FromRow, Debug)]
    struct PackageVersion {
        branch: String,
        fullver: String,
    }

    #[derive(FromRow)]
    struct ResUpstream {
        version: String,
        updated: i64,
        url: String,
    }

    #[derive(FromRow, Serialize, Debug)]
    struct PackageError {
        message: String,
        path: String,
        tree: String,
        branch: String,
    }

    #[derive(Template, Debug, Serialize)]
    #[template(path = "package.html")]
    struct Template<'a> {
        pkg: &'a Package,
        name: &'a String,
        version: &'a String,
        description: &'a String,
        tree: &'a String,
        category: &'a String,
        section: &'a String,
        dependencies: Vec<Dependency>,
        errors: Vec<PackageError>,
        hasrevdep: bool,
        srctype: String,
        srcurl_base: String,
        srcurl: String,
        hasupstream: bool,
        upstream_ver_compare: i64,
        upstream_url: String,
        upstream_updated: i64,
        upstream_version: String,
        full_version: &'a String,
        versions: Vec<Version>,
        version_matrix: Vec<MatrixRow>,
    }

    let mut pkg: Option<Package> = query_as(SQL_GET_PACKAGE_INFO)
        .bind(&name)
        .fetch_optional(&db.abbs)
        .await?;
    let mut pkgintree = true;

    if pkg.is_none() {
        pkg = query_as(SQL_GET_PACKAGE_INFO_GHOST)
            .bind(&name)
            .fetch_optional(&db.abbs)
            .await?;
        pkgintree = false;
    }

    let pkg = if let Some(pkg) = pkg {
        pkg
    } else {
        return not_found!("Package \"{}\" not found", name);
    };

    // collect package error messages
    let errors: Vec<PackageError> =
        query_as("SELECT message,path,tree,branch FROM package_errors WHERE package = ?")
            .bind(&name)
            .fetch_all(&db.abbs)
            .await?;

    // Generate version matrix
    let dpkgs: Vec<DpkgPackage> = query_as(SQL_GET_PACKAGE_DPKG)
        .bind(&name)
        .fetch_all(&db.abbs)
        .await?;

    // 1. collect vers list from dpkgs, sorted by desc
    let mut vers: HashSet<_> = dpkgs.iter().map(|x| x.version.clone()).collect();
    let fullver = &pkg.full_version;
    if pkgintree & !fullver.is_empty() & !vers.contains(fullver) {
        vers.insert(fullver.clone());
    }
    let vers = vers
        .into_iter()
        .sorted_by(|a, b| deb_version::compare_versions(a, b).reverse())
        .collect_vec();

    // 2. generate src_vers from SQL query
    let src_vers: Vec<PackageVersion> = query_as(SQL_GET_PACKAGE_VERSIONS)
        .bind(&name)
        .fetch_all(&db.abbs)
        .await?;
    let src_vers: HashMap<_, _> = src_vers
        .into_iter()
        .map(|v| (v.fullver, v.branch))
        .collect();

    // 3. generate versions list
    let versions= vers.iter().map(|version| {
        let branch = src_vers.get(version).cloned().unwrap_or_default();
        let url = if !branch.is_empty() {
            let (tree, section, directory) = (&pkg.tree, &pkg.section, &pkg.directory);
            let category = if !pkg.category.is_empty() {
                format!("{}-", pkg.category)
            } else {
                "".into()
            };
            format!(
                "https://github.com/AOSC-Dev/{tree}/tree/{branch}/{category}{section}/{directory}/spec"
            )
        } else {
            "".into()
        };

        Version { version: version.clone(), url, branch }
    }).collect_vec();

    // 4. generate reponames list, sorted by asc
    let reponames = if pkg.noarch {
        vec!["noarch".into()]
    } else if !pkg.tree_category.is_empty() & !pkg.fail_arch.is_empty() {
        let fail_arch =
            FailArch::from(&pkg.fail_arch).map_err(|_| anyhow!("invalid FAIL_ARCH format"))?;
        db_repos(&db)
            .await?
            .into_values()
            .filter_map(|r| {
                let flag = match &fail_arch {
                    FailArch::Include(v) => !v.contains(&r.realname),
                    FailArch::Exclude(v) => v.contains(&r.realname),
                };
                if (r.category == pkg.tree_category) & (r.realname != "noarch") & flag {
                    Some(r.realname)
                } else {
                    None
                }
            })
            .collect()
    } else {
        dpkgs.iter().map(|p| p.reponame.clone()).collect()
    };
    let reponames = reponames
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .sorted()
        .collect_vec();

    // 5. generate dpkg_matrix
    let dpkg_repos: HashSet<_> = dpkgs.iter().map(|d| &d.reponame).collect();
    let dpkg_matrix = reponames
        .iter()
        .map(|repo| {
            if !dpkg_repos.contains(repo) {
                let meta = repeat(DpkgMeta::default()).take(versions.len()).collect();
                let row = MatrixRow {
                    repo: repo.clone(),
                    meta,
                };
                return row;
            }

            let meta = versions
                .iter()
                .map(|ver| {
                    let dpkg = dpkgs
                        .iter()
                        .find(|d| (&d.reponame == repo) & (d.version == ver.version));

                    if let Some(dpkg) = dpkg {
                        let (size, testing, repo) = (dpkg.size, dpkg.testing, dpkg.repo.clone());
                        DpkgMeta {
                            hasmeta: true,
                            version: ver.version.clone(),
                            testing,
                            repo,
                            size,
                        }
                    } else {
                        DpkgMeta::default()
                    }
                })
                .collect_vec();

            MatrixRow {
                repo: repo.clone(),
                meta,
            }
        })
        .collect_vec();

    // deal with upstream related variables
    let mut upstream_url = String::new();
    let mut upstream_updated = 0i64;
    let mut upstream_version = String::new();
    let mut upstream_ver_compare = 0i64;
    let mut hasupstream = false;
    let res: Option<ResUpstream> = query_as(SQL_GET_PISS_VERSION)
        .bind(&name)
        .fetch_optional(&db.abbs)
        .await?;
    if let Some(res) = res {
        if !pkg.version.is_empty() & !res.version.is_empty() {
            upstream_url = res.url;
            upstream_updated = res.updated;
            upstream_version = res.version.clone();
            hasupstream = true;

            if res.version.starts_with(&pkg.version) {
                upstream_ver_compare = 0; // same
            } else {
                let cmp = deb_version::compare_versions(&pkg.version, &res.version);
                upstream_ver_compare = match cmp {
                    std::cmp::Ordering::Less => -1,   // old
                    std::cmp::Ordering::Equal => 0,   // same
                    std::cmp::Ordering::Greater => 1, // new
                };
            }
        }
    }

    // guess upstream url
    let res = Src::parse(&pkg.srctype, &pkg.srcurl);
    let srctype = res
        .as_ref()
        .map(|x| x.srctype.to_string())
        .unwrap_or_default();
    let srcurl = res
        .as_ref()
        .map(|x| x.srcurl.to_string())
        .unwrap_or_default();
    let srcurl_base = res
        .and_then(|Src { srcurl, srctype }| {
            if regex_srchost(&srcurl) {
                let v: Vec<_> = srcurl.split('/').take(5).collect();
                Some(v.join("/"))
            } else if regex_pypi(&srcurl) {
                let pypiname = if regex_pypisrc(&srcurl) {
                    srcurl.split('/').nth_back(2)
                } else {
                    srcurl
                        .split('/')
                        .nth_back(1)
                        .and_then(|s| s.rsplit_once('-').map(|x| x.0))
                };

                pypiname.map(|pypiname| format!("https://pypi.python.org/pypi/{pypiname}/"))
            } else {
                match srctype {
                    SrcType::Tarball => srcurl.split_once('/').map(|x| x.0.to_string()),
                    SrcType::Git => {
                        if let Some(stripped) = srcurl.strip_prefix("git://") {
                            Some(format!("http://{stripped}"))
                        } else {
                            Some(srcurl)
                        }
                    }
                    _ => None,
                }
            }
        })
        .unwrap_or_default();

    let ctx = Template {
        // package
        pkg: &pkg,
        name: &name,
        version: &pkg.version,
        description: &pkg.description,
        tree: &pkg.tree,
        category: &pkg.category,
        section: &pkg.section,
        hasrevdep: pkg.hasrevdep,
        full_version: &pkg.full_version,

        // dependencies
        dependencies: Dependency::parse_db_dependencies(&pkg.dependency),

        // errors
        errors,

        // dpkg_matrix
        versions,
        version_matrix: dpkg_matrix,

        // upstream
        srctype,
        srcurl_base,
        srcurl,
        upstream_ver_compare,
        upstream_url,
        upstream_updated,
        upstream_version,
        hasupstream,
    };

    render::<_, Template>(ctx, None, &q)
}

typed_path!(
    "/files/:reponame/:branch/:name/:version",
    Files,
    reponame,
    branch,
    name,
    version
);
pub async fn files(
    Files {
        reponame,
        branch,
        name,
        version,
    }: Files,
    q: Query,
    db: Ext,
) -> Result<impl IntoResponse> {
    let repo = format!("{reponame}/{branch}");

    #[derive(Debug, FromRow, Serialize)]
    struct Package {
        package: String,
        version: String,
        // architecture: String,
        repo: String,
        maintainer: String,
        installed_size: i64,
        filename: String,
        size: i64,
        sha256: String,
    }

    #[derive(Debug, FromRow)]
    struct DebTime {
        debtime: i32,
    }

    #[derive(Debug, FromRow)]
    struct SoDep {
        depends: i32,
        soname: Option<String>,
    }

    #[derive(Debug, FromRow, Serialize)]
    struct File {
        filename: Option<String>,
        size: i64,
        ftype: i16,
        perm: i32,
        uid: i64,
        gid: i64,
        uname: String,
        gname: String,
    }

    #[derive(Template, Debug, Serialize)]
    #[template(path = "files.html")]
    struct Template<'a> {
        files: &'a Vec<File>,
        sodepends: Vec<String>,
        soprovides: Vec<String>,
        pkg_debtime: i32,
        pkg: Package,
    }

    #[derive(Template, Debug)]
    #[template(path = "files.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        files: &'a Vec<File>,
    }

    let pkg: Option<Package> = query_as(SQL_GET_PACKAGE_DEB_LOCAL)
        .bind(&name)
        .bind(&version)
        .bind(&repo)
        .fetch_optional(&db.abbs)
        .await?;
    let pkg = if let Some(pkg) = pkg {
        pkg
    } else {
        return not_found!("Package \"{name}\" ({version}) not found in {repo}");
    };

    let pkg_debtime = query_as("SELECT debtime FROM pv_packages WHERE filename=$1")
        .bind(&pkg.filename)
        .fetch_optional(&db.pg)
        .await?
        .map_or(0, |d: DebTime| d.debtime);

    let files: &Vec<File> = &query_as(SQL_GET_PACKAGE_DEB_FILES)
        .bind(&name)
        .bind(&version)
        .bind(&repo)
        .fetch_all(&db.pg)
        .await?;

    // generate sodepends and soprovides list
    let sodep: Vec<SoDep> = query_as(SQL_GET_PACKAGE_SODEP)
        .bind(&name)
        .bind(&version)
        .bind(&repo)
        .fetch_all(&db.pg)
        .await?;
    let mut sodepends = vec![];
    let mut soprovides = vec![];
    for SoDep { depends, soname } in sodep {
        if let Some(soname) = soname {
            if depends != 0 {
                sodepends.push(soname)
            } else {
                soprovides.push(soname)
            }
        }
    }

    let ctx = Template {
        files,
        sodepends,
        soprovides,
        pkg_debtime,
        pkg,
    };

    let ctx_tsv = TemplateTsv { files };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/qa", RouteQa);
pub async fn qa(_: RouteQa) -> impl IntoResponse {
    Redirect::to("/qa/")
}

typed_path!("/qa/", Qa);
pub async fn qa_index(_: Qa, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(Debug, Serialize)]
    struct Package {
        package: String,
        version: String,
        errs: Vec<i32>,
    }

    #[derive(Debug, Serialize)]
    struct SrcIssuesMatrixRow {
        tree: String,
        branch: String,
        issues: Vec<Issue>,
    }

    #[derive(Debug, Serialize)]
    struct DebIssuesMatrixRow {
        arch: String,
        branch: String,
        oldcnt: i64,
        issues: Vec<Issue>,
    }

    #[derive(Debug, Default, Serialize)]
    struct Issue {
        errno: i32,
        cnt: i64,
        ratio: f64,
    }

    #[derive(Debug, FromRow)]
    struct IssueRes {
        repo: Option<String>,
        errno: Option<i32>,
        cnt: Option<i64>,
        ratio: Option<f64>,
    }

    #[derive(Debug, FromRow)]
    struct Recent {
        package: Option<String>,
        version: Option<String>,
        errs: Vec<i32>,
    }

    #[derive(Template, Debug, Serialize)]
    #[template(path = "qa_index.html")]
    struct Template<'a> {
        total: i64,
        percent: f64,
        recent: Vec<Package>,

        srcissues_max: f64,
        srcissues_key: Vec<i32>,
        srcissues_matrix: &'a Vec<SrcIssuesMatrixRow>,

        debissues_max: f64,
        debissues_key: Vec<i32>,
        debissues_matrix: &'a Vec<DebIssuesMatrixRow>,
    }

    #[derive(Template, Debug)]
    #[template(path = "qa_index.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        srcissues_matrix: &'a Vec<SrcIssuesMatrixRow>,
        debissues_matrix: &'a Vec<DebIssuesMatrixRow>,
    }

    #[derive(Debug, FromRow)]
    struct OldDeb {
        repo: Option<String>,
        oldcnt: Option<i64>,
    }

    #[derive(Debug, FromRow)]
    struct TreeBranch {
        tree: String,
        branch: String,
    }

    let tree_branches: Vec<TreeBranch> = query_as("SELECT name, tree, branch FROM tree_branches")
        .fetch_all(&db.abbs)
        .await?;
    let repos = db_repos(&db).await?;

    let olddebs: Vec<OldDeb> = query_as("SELECT repo, oldcnt FROM dpkg_repo_stats")
        .fetch_all(&db.abbs)
        .await?;

    let issues: Vec<IssueRes> = query_as(SQL_ISSUES_STATS).fetch_all(&db.pg).await?;
    let mut total = 0;
    let mut percent = 0.0;

    let mut src_issues = HashMap::new(); //Hashmap<(arch,branch,HashMap<errno,(cnt,ratio)>)>
    let mut srcissues_key = HashSet::new();
    let mut srcissues_max = 0.0;

    let mut deb_issues = HashMap::new(); //Hashmap<(arch,branch,HashMap<errno,(cnt,ratio)>)>
    let mut debissues_key = HashSet::new();
    let mut debissues_max = 0.0;

    for issue in issues {
        if issue.repo.is_none() & issue.errno.is_none() {
            total = issue.cnt.unwrap_or_default();
            percent = issue.ratio.unwrap_or_default();
            continue;
        }

        let errno = skip_none!(issue.errno);
        let repo = skip_none!(issue.repo);
        let cnt = issue.cnt.unwrap_or_default();
        let ratio = issue.ratio.unwrap_or_default();

        if (errno < 200) | (400..=409).contains(&errno) {
            // src issue
            let (repo, branch) = skip_none!(repo.split_once('/'));
            src_issues
                .entry((repo.to_string(), branch.to_string()))
                .or_insert_with(HashMap::new)
                .insert(errno, (cnt, ratio));
            srcissues_key.insert(errno);
            srcissues_max = ratio.max(srcissues_max);
        } else {
            // deb issue
            if let Some(r) = repos.get(&repo) {
                deb_issues
                    .entry((&r.architecture, &r.branch))
                    .or_insert_with(HashMap::new)
                    .insert(errno, (cnt, ratio));
                debissues_key.insert(errno);
                debissues_max = ratio.max(debissues_max);
            }
        }
    }

    let srcissues_key = srcissues_key.into_iter().sorted().collect_vec();
    let srcissues_matrix = &tree_branches
        .iter()
        .filter_map(|r| {
            if let Some(errs) = src_issues.get(&(r.tree.clone(), r.branch.clone())) {
                let issues = srcissues_key
                    .iter()
                    .map(|err| {
                        if let Some((cnt, ratio)) = errs.get(err) {
                            Issue {
                                errno: *err,
                                ratio: *ratio,
                                cnt: *cnt,
                            }
                        } else {
                            Issue::default()
                        }
                    })
                    .collect_vec();

                let row = SrcIssuesMatrixRow {
                    tree: r.tree.clone(),
                    branch: r.branch.clone(),
                    issues,
                };
                Some(row)
            } else {
                None
            }
        })
        .collect_vec();

    let debissues_key = debissues_key.into_iter().sorted().collect_vec();
    let debissues_matrix = &repos
        .values()
        .filter_map(|r| {
            if let Some(errs) = deb_issues.get(&(&r.architecture, &r.branch)) {
                let issues = debissues_key
                    .iter()
                    .map(|err| {
                        if let Some((cnt, ratio)) = errs.get(err) {
                            Issue {
                                errno: *err,
                                ratio: *ratio,
                                cnt: *cnt,
                            }
                        } else {
                            Issue::default()
                        }
                    })
                    .collect_vec();

                let oldcnt = olddebs
                    .iter()
                    .find(|x| x.repo.as_ref() == Some(&r.name))
                    .map_or(0, |x| x.oldcnt.unwrap_or_default());

                let row = DebIssuesMatrixRow {
                    arch: r.architecture.clone(),
                    branch: r.branch.clone(),
                    oldcnt,
                    issues,
                };
                Some(row)
            } else {
                None
            }
        })
        .collect_vec();

    // recent packages
    let recent = query_as(SQL_ISSUES_RECENT)
        .fetch_all(&db.pg)
        .await?
        .into_iter()
        .filter_map(|p: Recent| {
            if let Some(package) = p.package {
                Some(Package {
                    package,
                    version: p.version.unwrap_or_default(),
                    errs: p.errs,
                })
            } else {
                None
            }
        })
        .collect_vec();

    let ctx = Template {
        total,
        percent,
        recent,
        srcissues_max,
        srcissues_key,
        srcissues_matrix,
        debissues_max,
        debissues_key,
        debissues_matrix,
    };

    let ctx_tsv = TemplateTsv {
        srcissues_matrix,
        debissues_matrix,
    };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/qa/code/:code", QaCode, code);
pub async fn qa_code(QaCode { code }: QaCode, q: Query, db: Ext) -> Result<Response> {
    qa_code_common(code, None, q, db).await
}

typed_path!("/qa/code/:code/*repo", QaRepo, code, repo);
pub async fn qa_repo(QaRepo { code, repo }: QaRepo, q: Query, db: Ext) -> Result<Response> {
    qa_code_common(code, Some(repo), q, db).await
}

async fn qa_code_common(code: String, repo: Option<String>, q: Query, db: Ext) -> Result<Response> {
    #[derive(Debug, FromRow, Serialize)]
    struct Package {
        name: String,
        versions: Vec<String>,
        branches: Vec<String>,
        filename: Option<String>,
        filecount: i64,
    }

    #[derive(Debug, Template, Serialize)]
    #[template(path = "qa_code.html")]
    struct Template<'a> {
        code: i32,
        repo: String,
        description: String,
        packages: &'a [Package],
        page: Page,
    }

    #[derive(Debug, Template)]
    #[template(path = "qa_code.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        packages: &'a [Package],
    }

    let repo = if let Some(repo) = repo {
        Some(strip_prefix(&repo)?.to_string())
    } else {
        None
    };

    if let Some(repo) = &repo {
        let res = query(
            "
        SELECT name FROM dpkg_repos WHERE name=? UNION ALL 
        SELECT name FROM tree_branches WHERE name=?",
        )
        .bind(&repo)
        .bind(&repo)
        .fetch_optional(&db.abbs)
        .await?;

        if res.is_none() {
            return not_found!("Repo \"{repo}\" not found.");
        }
    }

    let code = code
        .parse::<i32>()
        .with_context(|| format!("Issue code \"{code}\" not found."))?;
    let description = issue_code(code)
        .with_context(|| format!("Issue code \"{code}\" not found."))?
        .to_string();

    let mut pkgs: Vec<Package> = query_as(SQL_ISSUES_CODE)
        .bind(&code)
        .bind(&repo)
        .fetch_all(&db.pg)
        .await?;

    let (page, pkgs) = if let Some(cur) = q.get_page() {
        let ceil = |a, b| (a + b - 1) / b;
        let page = Page {
            cur,
            max: ceil(pkgs.len() as u32, PAGESIZE),
            count: pkgs.len() as u32,
        };
        let pkgs = pkgs
            .chunks_mut(PAGESIZE as usize)
            .nth(cur as usize - 1)
            .with_context(|| "page param out of range")?;
        pkgs.iter_mut().for_each(|pkg| pkg.versions.sort());

        (page, pkgs)
    } else {
        let page = Page {
            cur: 1,
            max: 0,
            count: pkgs.len() as u32,
        };
        (page, &mut pkgs[..])
    };

    let ctx = Template {
        code,
        repo: repo.unwrap_or_default(),
        description,
        packages: pkgs,
        page,
    };

    let ctx_tsv = TemplateTsv { packages: pkgs };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/qa/packages/:name", QaPkg, name);
pub async fn qa_package(QaPkg { name }: QaPkg, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Debug)]
    struct Package {
        tree: String,
        category: String,
        section: String,
        description: String,
        version: String,
        dependency: String,
        hasrevdep: bool,
    }

    #[derive(FromRow, Debug)]
    struct PkgIssue {
        errno: i32,
        version: String,
        repo: String,
        filecount: i64,
        level: i16,
        filename: Option<String>,
        detail: Option<serde_json::Value>,
    }

    #[derive(Debug, Serialize)]
    struct Issue {
        errno: i32,

        examples: Vec<Example>,
    }

    #[derive(Debug, Serialize)]
    struct Info {
        file: File,
        detail: serde_json::Value,
    }

    #[derive(Debug, Serialize)]
    struct Example {
        filecount: usize,
        info: Vec<Info>,
        ver_repo: Vec<(String, String)>, // Vec<(version,repo)>
        level: i16,

        // for errno 421 432 431
        custom: serde_json::Value,
    }

    #[derive(Debug, Serialize)]
    struct File {
        filename: String,
    }

    #[derive(Debug, Template, Serialize)]
    #[template(path = "qa_package.html")]
    struct Template {
        name: String,
        version: String,
        description: String,
        tree: String,
        category: String,
        section: String,
        hasrevdep: bool,
        dependencies: Vec<Dependency>,
        issues: Vec<Issue>,
    }

    let name = name
        .split('#')
        .next()
        .with_context(|| format!("cannot split package name {name}"))?;

    let mut pkg: Option<Package> = query_as(SQL_GET_PACKAGE_INFO)
        .bind(name)
        .fetch_optional(&db.abbs)
        .await?;
    if pkg.is_none() {
        pkg = query_as(SQL_GET_PACKAGE_INFO_GHOST)
            .bind(name)
            .fetch_optional(&db.abbs)
            .await?;
    }
    let pkg = if let Some(pkg) = pkg {
        pkg
    } else {
        return not_found!("Package \"{name}\" not found.");
    };

    let issues: Vec<PkgIssue> = query_as(SQL_ISSUES_PACKAGE)
        .bind(name)
        .fetch_all(&db.pg)
        .await?;

    let errno_examples = issues
        .iter()
        .group_by(|i| (i.errno, &i.version, &i.repo))
        .into_iter()
        .map(|((errno, _version, _repo), group)| {
            let issues = group.collect_vec();
            let filecount = issues[0].filecount as usize;
            let level = issues[0].level;

            let mut ver_repo = IndexSet::new();
            let mut info = vec![];
            for issue in &issues {
                ver_repo.insert((issue.version.clone(), issue.repo.clone()));

                if [421, 432, 431].contains(&errno) {
                    continue;
                } else {
                    info.push(Info {
                        file: File {
                            filename: issue.filename.clone().unwrap_or_default(),
                        },
                        detail: issue.detail.clone().unwrap_or_default(),
                    });
                }
            }

            let custom = match errno {
                421 | 431 | 432 => {
                    #[derive(Debug, Serialize)]
                    struct Custom {
                        summary: Vec<String>,
                        files_bypkg: Vec<FileByPkg>,
                    }

                    #[derive(Debug, Serialize)]
                    struct FileByPkg {
                        pkg_name: String,
                        repo: String,
                        version: String,
                        files: Vec<String>,
                    }

                    let mut summary = IndexSet::new();
                    let mut files_bypkg = IndexMap::new();
                    for PkgIssue {
                        filename, detail, ..
                    } in issues
                    {
                        let filename = skip_none!(filename);
                        let detail = skip_none!(detail);

                        let get = |key| detail.get(key).and_then(|pkg| pkg.as_str());
                        if let Some(pkg) = get("package") {
                            summary.insert(pkg.to_string());

                            let filename = if errno == 431 {
                                if let Some(sover_provide) = get("sover_provide") {
                                    let sover_provide = sover_provide.trim_start_matches('.');
                                    format!("{filename} (provided: {sover_provide})")
                                } else {
                                    filename.to_string()
                                }
                            } else {
                                filename.to_string()
                            };

                            if let (Some(repo), Some(version)) = (get("repo"), get("version")) {
                                files_bypkg
                                    .entry((pkg.to_string(), repo.to_string(), version.to_string()))
                                    .or_insert(vec![])
                                    .push(filename);
                            }
                        }
                    }

                    let files_bypkg = files_bypkg
                        .into_iter()
                        .map(|((pkg_name, repo, version), files)| FileByPkg {
                            pkg_name,
                            repo,
                            version,
                            files,
                        })
                        .collect_vec();

                    serde_json::to_value(Custom {
                        summary: summary.into_iter().collect_vec(),
                        files_bypkg: files_bypkg.into_iter().collect_vec(),
                    })
                    .unwrap_or_default()
                }
                _ => serde_json::Value::Null,
            };

            (
                errno,
                Example {
                    filecount,
                    info,
                    custom,
                    ver_repo: ver_repo.into_iter().collect_vec(),
                    level,
                },
            )
        })
        .collect_vec();

    let mut issues: Vec<Issue> = vec![];
    for (errno, example) in errno_examples {
        if let Some(i) = issues.last_mut() {
            if i.errno == errno {
                i.examples.push(example);
                continue;
            }
        }

        issues.push(Issue {
            errno,
            examples: vec![example],
        });
    }

    let ctx = Template {
        name: name.to_string(),
        version: pkg.version,
        description: pkg.description,
        tree: pkg.tree,
        category: pkg.category,
        section: pkg.section,
        dependencies: Dependency::parse_db_dependencies(&pkg.dependency),
        hasrevdep: pkg.hasrevdep,
        issues,
    };

    render::<_, Template>(ctx, None, &q)
}

typed_path!("/cleanmirror/*repo", CleanMirror, repo);
pub async fn cleanmirror(
    CleanMirror { repo }: CleanMirror,
    q: Query,
    db: Ext,
) -> Result<impl IntoResponse> {
    let reason: Option<HashSet<_>> = q
        .get_reason()
        .as_ref()
        .map(|r| r.split(',').map(|x| x.to_string()).collect());
    let repo = strip_prefix(&repo)?;

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

typed_path!("/revdep/:name", Revdep, name);
pub async fn revdep(Revdep { name }: Revdep, q: Query, db: Ext) -> Result<impl IntoResponse> {
    let res = query("SELECT 1 FROM packages WHERE name = ?")
        .bind(&name)
        .fetch_optional(&db.abbs)
        .await?;
    if res.is_none() {
        return not_found!("Package \"{name}\" not found.");
    }

    #[derive(Debug, FromRow, Serialize)]
    struct RevDep {
        package: String,
        version: String,
        relationship: String,
        architecture: String,
    }

    #[derive(Debug, Serialize)]
    struct TemplateRevDep<'a> {
        description: &'a str,
        deps: Vec<&'a &'a RevDep>,
    }

    #[derive(Debug, Template, Serialize)]
    #[template(path = "revdep.html")]
    struct Template<'a> {
        name: &'a String,

        revdeps: &'a Vec<TemplateRevDep<'a>>,
        sobreaks: &'a Vec<Vec<String>>,
        sobreaks_circular: &'a Vec<String>,
    }

    #[derive(Debug, Template, Serialize)]
    #[template(path = "revdep.tsv", escape = "none")]
    struct TemplateTsv<'a> {
        revdeps: &'a Vec<TemplateRevDep<'a>>,
        sobreaks: &'a Vec<Vec<String>>,
        sobreaks_circular: &'a Vec<String>,
    }

    #[derive(Debug, FromRow)]
    struct Sobreak {
        dep_package: String,
        deplist: Vec<String>,
    }

    let deps: Vec<RevDep> = query_as(SQL_GET_PACKAGE_REV_REL)
        .bind(&name)
        .fetch_all(&db.abbs)
        .await?;

    let deps_map: IndexMap<_, _> = deps
        .iter()
        .group_by(|dep| &dep.relationship)
        .into_iter()
        .map(|(k, v)| (k, v.collect_vec()))
        .collect();

    let revdeps = &DEP_REL_REV
        .iter()
        .filter_map(|(relationship, description)| {
            if let Some(deps) = deps_map.get(&relationship.to_string()) {
                let mut res = vec![];
                for (_, pkggroup) in &deps.iter().group_by(|dep| &dep.package) {
                    let mut iter = pkggroup;
                    if let Some(dep) = iter.find(|dep| dep.architecture.is_empty()) {
                        res.push(dep);
                    } else {
                        res.append(&mut iter.collect_vec());
                    }
                }
                Some(TemplateRevDep {
                    description,
                    deps: res,
                })
            } else {
                None
            }
        })
        .collect_vec();

    let sobreaks: Vec<Sobreak> =
        query_as("SELECT dep_package, deplist FROM v_so_breaks_dep WHERE package=$1")
            .bind(&name)
            .fetch_all(&db.pg)
            .await?;

    let toposort = |sobreaks: Vec<Sobreak>| {
        let mut data: HashMap<_, _> = sobreaks
            .into_iter()
            .map(|p| (p.dep_package, HashSet::from_iter(p.deplist)))
            .collect();
        let mut sobreaks = vec![];

        loop {
            let ordered: HashSet<_> = data
                .iter()
                .filter(|(_, dep)| dep.is_empty())
                .map(|(pkg, _)| pkg.to_string())
                .collect();

            if ordered.is_empty() {
                break;
            }

            data = data
                .into_iter()
                .filter_map(|(item, dep)| {
                    if !ordered.contains(&item) {
                        Some((item, &dep - &ordered))
                    } else {
                        None
                    }
                })
                .collect();

            sobreaks.push(ordered.into_iter().sorted().collect_vec());
        }

        let circular = data.into_keys().collect_vec();
        sobreaks.reverse();
        (sobreaks, circular)
    };

    let (ref sobreaks, ref sobreaks_circular) = toposort(sobreaks);

    let ctx = Template {
        name: &name,
        revdeps,
        sobreaks,
        sobreaks_circular,
    };

    let ctx_tsv = TemplateTsv {
        revdeps,
        sobreaks,
        sobreaks_circular,
    };

    render(ctx, Some(ctx_tsv), &q)
}
