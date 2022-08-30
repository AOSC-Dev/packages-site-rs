use crate::db::Db;
use crate::sql::SQL_GET_REPO_COUNT;
use crate::sql::SQL_GET_TREES;
use anyhow::Context;
use askama::Template;
use axum::async_trait;
use axum::extract::FromRequest;
use axum::extract::RequestParts;
use axum::http;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Extension;
use indexmap::IndexMap;
use proc_macro_regex::regex;
use serde::Deserialize;
use serde::Serialize;
use sqlx::query_as;
use sqlx::FromRow;
use std::collections::HashMap;
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, Error>;
pub type Ext = Extension<Arc<Db>>;

macro_rules! typed_path {
    ($path:literal,$name:ident) => {
        #[derive(serde::Deserialize,axum_macros::TypedPath)]
        #[typed_path($path)]
        pub struct $name{}
    };
    ($path:literal,$name:ident,$($field:ident),+ $(,)?) => {
        #[derive(serde::Deserialize,axum_macros::TypedPath)]
        #[typed_path($path)]
        pub struct $name {
            $(
                $field:String,
            )*
        }
    }
}

#[inline(always)]
pub fn build_resp<V, T>(mime: V, body: T) -> Result<Response<T>>
where
    HeaderValue: TryFrom<V>,
    <HeaderValue as TryFrom<V>>::Error: Into<axum::http::Error>,
{
    Ok(Response::builder().header(header::CONTENT_TYPE, mime).body(body)?)
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("404 Not Found: {0}")]
    NotFound(String),
    #[error("Not Supported: {0}")]
    NotSupported(String),
    #[error(transparent)]
    Http(#[from] axum::http::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Query(#[from] axum::extract::rejection::QueryRejection),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        #[derive(Template)]
        #[template(path = "error.html")]
        struct Template {
            error: String,
        }

        let ctx = Template {
            error: self.to_string(),
        };
        let status_code = match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (status_code, into_response(&ctx, None)).into_response()
    }
}

macro_rules! not_found {
    ($($arg:tt)*) => {
        Err(Error::NotFound(format!($($arg)*)))
    };
}

pub(crate) use not_found;
pub(crate) use typed_path;

pub fn into_response<T: Template>(t: &T, mine: Option<&'static str>) -> Response {
    match t.render() {
        Ok(body) => {
            let headers = [(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(mine.unwrap_or(T::MIME_TYPE)),
            )];

            (headers, body).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub fn render<T: Template + Serialize, V: Template>(ctx: T, ctx_tsv: Option<V>, q: &Query) -> Result<Response> {
    Ok(match q.get_type() {
        Some("tsv") => {
            if let Some(ctx_tsv) = ctx_tsv {
                into_response(&ctx_tsv, Some(mime_guess::mime::TEXT_PLAIN.as_ref()))
            } else {
                Error::NotSupported("cannot render current page into tsv format".to_string()).into_response()
            }
        }
        Some("json") => build_resp(mime_guess::mime::JSON.as_ref(), serde_json::to_string(&ctx)?).into_response(),
        _ => into_response(&ctx, None),
    })
}

pub async fn fallback(uri: Uri) -> impl IntoResponse {
    crate::utils::Error::NotFound(format!("No route for {}", uri))
}

pub fn issue_code(code: i32) -> Option<&'static str> {
    match code {
        100 => Some("Metadata"),
        101 => Some("Syntax error(s) in spec"),
        102 => Some("Syntax error(s) in defines"),
        103 => Some("Package name is not valid"),
        111 => Some("Package may be out-dated"),
        112 => Some("SRCTBL uses HTTP"),
        121 => Some("The last commit message was badly formatted"),
        122 => Some("Multiple packages changed in the last commit"),
        123 => Some("Force-pushed recently (last N commit - TBD)"),
        200 => Some("Build Process"),
        201 => Some("Failed to get source"),
        202 => Some("Failed to get dependencies"),
        211 => Some("Failed to build from source (FTBFS)"),
        221 => Some("Failed to launch packaged executable(s)"),
        222 => Some("Feature(s) non-functional, or unit test(s) failed"),
        300 => Some("Payload (.deb Package)"),
        301 => Some("Bad or corrupted .deb file"),
        302 => Some(".deb file too small"),
        303 => Some("Bad .deb filename or storage path"),
        311 => Some("Bad .deb Maintainer metadata"),
        321 => Some("File(s) stored in unexpected path(s) in .deb"),
        322 => Some("Zero-byte file(s) found in .deb"),
        323 => Some("File(s) with bad owner/group found in .deb"),
        324 => Some("File(s) with bad permission found in .deb"),
        400 => Some("Dependencies"),
        401 => Some("BUILDDEP unmet"),
        402 => Some("Duplicate package in tree"),
        411 => Some("PKGDEP unmet"),
        412 => Some("Duplicate package in repository"),
        421 => Some("File collision(s)"),
        431 => Some("Library version (sover) dependency unmet"),
        432 => Some("Library dependency without PKGDEP"),
        _ => None,
    }
}

pub type Query = QueryExtractor;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct QueryExtractor {
    page: Option<String>,
    q: Option<String>,
    noredir: Option<bool>,
    reason: Option<String>,
    r#type: Option<String>,
}

#[async_trait]
impl<B> FromRequest<B> for QueryExtractor
where
    B: Send,
{
    type Rejection = Error;

    async fn from_request(req: &mut RequestParts<B>) -> std::result::Result<Self, Self::Rejection> {
        use axum::extract::Query;
        let mut res = Query::<QueryExtractor>::from_request(req).await?.0;

        if req
            .headers()
            .get("X-Requested-With")
            .and_then(|h| h.eq("XMLHttpRequest").then_some(()))
            .is_some()
        {
            res.r#type = Some("json".into());
        }

        Ok(res)
    }
}

impl QueryExtractor {
    #[inline(always)]
    pub fn get_type(&self) -> Option<&str> {
        if let Some(ref t) = self.r#type {
            Some(t.as_str())
        } else {
            None
        }
    }
    #[inline(always)]
    pub fn get_page(&self) -> Option<u32> {
        if let Some(ref page) = self.page {
            match page.as_str() {
                "all" => None,
                s => Some(s.parse::<u32>().unwrap_or(1)),
            }
        } else {
            Some(1)
        }
    }

    pub fn get_query(&self) -> &Option<String> {
        &self.q
    }

    pub fn get_noredir(&self) -> bool {
        self.noredir.unwrap_or(false)
    }

    pub fn get_reason(&self) -> &Option<String> {
        &self.reason
    }
}

pub fn strip_prefix(s: &str) -> Result<&str> {
    Ok(s.strip_prefix('/')
        .with_context(|| format!("falied to strip prefix \"{}\"", s))?)
}

pub async fn get_repo(repo: &str, db: &Ext) -> Result<Repo> {
    let repos = db_repos(db).await?;
    if let Some(repo) = repos.get(repo) {
        Ok(repo.clone())
    } else {
        not_found!("Repo \"{repo}\" not found.")
    }
}

pub async fn db_last_modified(db: Ext) -> Result<i64> {
    #[derive(Debug, FromRow)]
    struct CommitTime {
        commit_time: i64,
    }

    let res: Option<CommitTime> =
        query_as("SELECT commit_time FROM package_versions ORDER BY commit_time DESC LIMIT 1")
            .fetch_optional(&db.abbs)
            .await?;

    Ok(res.map(|t| t.commit_time).unwrap_or_default())
}

#[derive(FromRow, Debug, Clone, Serialize)]
#[allow(unused)]
pub struct Repo {
    pub name: String,
    pub realname: String,
    pub architecture: String,
    pub branch: String,
    pub date: i64,
    pub testing: i64,
    pub category: String,
    pub testingonly: i64,
    pub pkgcount: i64,
    pub ghost: i64,
    pub lagging: i64,
    pub missing: i64,
}

pub async fn db_repos(db: &Ext) -> Result<IndexMap<String, Repo>> {
    let repos: Vec<Repo> = query_as(SQL_GET_REPO_COUNT).fetch_all(&db.abbs).await?;

    let res = repos.into_iter().map(|repo| (repo.name.clone(), repo)).collect();

    Ok(res)
}

#[derive(FromRow, Debug, Clone, Serialize)]
#[allow(unused)]
pub struct Tree {
    pub name: String,
    pub category: String,
    pub url: String,
    pub date: i64,
    pub pkgcount: i64,
}
pub async fn db_trees(db: &Ext) -> Result<IndexMap<String, Tree>> {
    let trees: Vec<Tree> = query_as(SQL_GET_TREES).fetch_all(&db.abbs).await?;

    let res = trees.into_iter().map(|tree| (tree.name.clone(), tree)).collect();

    Ok(res)
}

pub fn ver_rel(ver_compare: i64) -> &'static str {
    match ver_compare {
        -2 => "deprecated",
        -1 => "old",
        0 => "same",
        1 => "new",
        _ => "unknown",
    }
}

#[derive(Debug, Serialize)]
pub struct Dependency {
    pub relationship: String,
    pub arch: String,
    pub packages: Vec<(String, String)>,
}

impl Dependency {
    pub fn parse_db_dependencies(s: &str) -> Vec<Self> {
        let mut map: HashMap<String, HashMap<String, Vec<(String, String)>>> = HashMap::new();
        for s in s.split(',') {
            let mut iter = s.split('|');

            let name = iter.next().unwrap_or("");
            let relop = iter.next().unwrap_or("");
            let relationship = iter.next().unwrap_or("");
            let arch = iter.next().unwrap_or("");

            let inner = map.entry(relationship.into()).or_insert_with(HashMap::new);
            inner
                .entry(arch.into())
                .or_insert(vec![])
                .push((name.into(), relop.into()));
        }

        let mut res = vec![];

        for (rel, display_rel) in DEP_REL.iter() {
            if let Some(rel) = map.get(*rel) {
                let mut v: Vec<_> = rel.keys().collect();
                v.sort();

                for arch in v {
                    let mut pkgs = rel[arch].clone();
                    pkgs.sort_by(|a, b| a.0.cmp(&b.0));

                    let dep = Self {
                        relationship: display_rel.to_string(),
                        arch: arch.clone(),
                        packages: pkgs,
                    };

                    res.push(dep);
                }
            }
        }

        res
    }
}

pub struct Src {
    pub srcurl: String,
    pub srctype: SrcType,
}

impl Src {
    pub fn parse(srctype: &str, srcurl: &str) -> Option<Self> {
        let (srctype, srcurl) = match srctype {
            "SRCS" => {
                // we only take the first set
                // see https://wiki.aosc.io/developer/packaging/acbs/spec-format/
                let srcurl = srcurl.split(' ').next()?;
                let params: Vec<_> = srcurl.split("::").collect();

                let (srctype, srcurl) = match params.len() {
                    1 => ("tbl", params[0]),
                    2 => (params[0], params[1]),
                    3 => (params[0], params[2]), // ignore options
                    _ => return None,
                };

                let srctype = match srctype {
                    "git" => SrcType::Git,
                    "svn" => SrcType::SvnSrc,
                    "tbl" => SrcType::Tarball,
                    "bzr" => SrcType::BzrSrc,
                    _ => return None,
                };

                (srctype, srcurl)
            }
            "SRCTBL" => (SrcType::Tarball, srcurl),
            "GITSRC" => (SrcType::Git, srcurl),
            "SVNSRC" => (SrcType::SvnSrc, srcurl),
            "BZRSRC" => (SrcType::BzrSrc, srcurl),
            _ => return None,
        };

        if !srcurl.is_empty() {
            Some(Self {
                srcurl: srcurl.to_string(),
                srctype,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SrcType {
    Git,
    Tarball,
    SvnSrc,
    BzrSrc,
}

impl std::fmt::Display for SrcType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SrcType::Git => "git",
            SrcType::Tarball => "tarball",
            SrcType::SvnSrc => "Subversion",
            SrcType::BzrSrc => "Bazaar",
        };
        write!(f, "{s}")
    }
}

macro_rules! skip_none {
    ($res:expr) => {
        match $res {
            Some(val) => val,
            None => {
                tracing::debug!("skip none");
                continue;
            }
        }
    };
}
pub(crate) use skip_none;

pub const REPO_CAT: [(&str, &str); 3] = [("base", ""), ("bsp", "BSP"), ("overlay", "Overlay")];
const DEP_REL: [(&str, &str); 8] = [
    ("PKGDEP", "Depends"),
    ("BUILDDEP", "Depends (build)"),
    ("PKGREP", "Replaces"),
    ("PKGRECOM", "Recommends"),
    ("PKGCONFL", "Conflicts"),
    ("PKGBREAK", "Breaks"),
    ("PKGPROV", "Provides"),
    ("PKGSUG", "Suggests"),
];
pub const DEP_REL_REV: [(&str, &str); 4] = [
    ("PKGDEP", "Depended by"),
    ("BUILDDEP", "Depended by (build)"),
    ("PKGRECOM", "Recommended by"),
    ("PKGSUG", "Suggested by"),
];

regex!(pub regex_srchost r"^https://(github\.com|bitbucket\.org|gitlab\.com)");
regex!(pub regex_pypi r"^https?://pypi\.(python\.org|io)");
regex!(pub regex_pypisrc r"^https?://pypi\.(python\.org|io)/packages/source/");
