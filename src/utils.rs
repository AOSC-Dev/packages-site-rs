use crate::db::Db;
use anyhow::Context;
use askama::Template;
use axum::async_trait;
use axum::extract::FromRequest;
use axum::extract::RequestParts;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Extension;
use indexmap::IndexMap;
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
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .body(body)?)
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

        (status_code, ctx).into_response()
    }
}

macro_rules! not_found {
    ($($arg:tt)*) => {
        Err(Error::NotFound(format!($($arg)*)))
    };
  }

pub(crate) use not_found;
pub(crate) use typed_path;

#[inline]
pub fn render<T: Template + Serialize + IntoResponse, V: Template + IntoResponse>(
    ctx: T,
    ctx_tsv: Option<V>,
    q: &Query,
) -> Result<Response> {
    Ok(match q.get_type() {
        Some("tsv") => {
            if let Some(ctx_tsv) = ctx_tsv {
                let body = ctx_tsv.into_response();
                build_resp(mime_guess::mime::TEXT_PLAIN.as_ref(), body).into_response()
            } else {
                Error::NotSupported("cannot render current page into tsv format".to_string())
                    .into_response()
            }
        }
        Some("json") => build_resp(
            mime_guess::mime::JSON.as_ref(),
            serde_json::to_string(&ctx)?,
        )
        .into_response(),
        _ => ctx.into_response(),
    })
}

/// askama user defined filters
pub mod filters {
    use std::fmt::Display;

    use itertools::Itertools;
    use serde_json::Value;

    use super::{issue_code, ver_rel};

    pub fn d<'a>(s: &'a str, default: &'a str, _: bool) -> ::askama::Result<&'a str> {
        if !s.is_empty() {
            Ok(s)
        } else {
            Ok(default)
        }
    }

    pub fn fmt_timestamp(timestamp: &i64) -> ::askama::Result<String> {
        if let Ok(datetime) = time::OffsetDateTime::from_unix_timestamp(*timestamp) {
            if let Ok(date) = datetime.format(&time::format_description::well_known::Rfc2822) {
                return Ok(date);
            }
        }

        return Err(askama::Error::Custom(
            anyhow::anyhow!("cannot format timestamp {timestamp} into RFC2822 format").into(),
        ));
    }

    pub fn cut(s: &str, len: usize) -> ::askama::Result<&str> {
        if s.len() <= len {
            Ok(s)
        } else {
            Ok(&s[..len])
        }
    }

    pub fn fill(s: &str, width: usize, subsequent_indent: &str) -> ::askama::Result<String> {
        let opt = textwrap::Options::new(width).subsequent_indent(subsequent_indent);
        Ok(textwrap::fill(s, opt))
    }

    pub fn get_first_line(s: &str) -> ::askama::Result<&str> {
        Ok(s.lines().next().unwrap_or(""))
    }

    pub fn strftime(timestamp: &i64, s: &str) -> ::askama::Result<String> {
        match time::OffsetDateTime::from_unix_timestamp(*timestamp) {
            Ok(datetime) => match time::format_description::parse(s) {
                Ok(fmt) => match datetime.format(&fmt) {
                    Ok(res) => Ok(res),
                    Err(e) => Err(askama::Error::Custom(Box::new(e))),
                },
                Err(e) => Err(askama::Error::Custom(Box::new(e))),
            },
            Err(e) => Err(askama::Error::Custom(Box::new(e))),
        }
    }

    pub fn calc_color_ratio(ratio: &f64, max: &f64) -> ::askama::Result<f64> {
        Ok(100.0 - 100.0 / 3.0 * (*ratio) / (*max))
    }

    pub fn strftime_i32(timestamp: &i32, s: &str) -> ::askama::Result<String> {
        strftime(&(*timestamp as i64), s)
    }

    pub fn sizeof_fmt(size: &i64) -> ::askama::Result<String> {
        let size = size::Size::from_bytes(*size);
        Ok(size.to_string())
    }

    pub fn fmt_ver_compare(ver_compare: &i64) -> ::askama::Result<&'static str> {
        Ok(ver_rel(*ver_compare))
    }

    pub fn fmt_issue_code(code: &i32) -> ::askama::Result<&'static str> {
        Ok(issue_code(*code).unwrap_or("unknown"))
    }

    pub fn sizeof_fmt_ls(num: &i64) -> ::askama::Result<String> {
        if num.abs() < 1024 {
            return Ok(num.to_string());
        }

        let mut num = (*num as f64) / 1024.0;

        for unit in "KMGTPEZ".chars() {
            if num.abs() < 10.0 {
                return Ok(format!("{num:.1}{unit}"));
            } else if num.abs() < 1024.0 {
                return Ok(format!("{num:.0}{unit}"));
            }
            num /= 1024.0
        }

        Ok(format!("{num:.1}Y"))
    }

    pub fn ls_perm(perm: &i32, ftype: &i16) -> ::askama::Result<String> {
        // see https://docs.rs/tar/latest/src/tar/entry_type.rs.html#70-87
        let ftype = match ftype {
            1 => 'l',
            3 => 'c',
            4 => 'b',
            5 => 'd',
            6 => 'p',
            _ => '-',
        };

        let perm: String = format!("{perm:b}")
            .chars()
            .zip("rwxrwxrwx".chars())
            .map(|(a, b)| if a == '1' { b } else { '-' })
            .collect();

        Ok(format!("{ftype}{perm}"))
    }

    pub fn ls_perm_str(perm: &i32, ftype: &str) -> ::askama::Result<String> {
        let ftype = match ftype {
            "lnk" => 'l',
            "sock" => 's',
            "chr" => 'c',
            "blk" => 'b',
            "dir" => 'd',
            "fifo" => 'p',
            _ => '-',
        };

        let perm: String = format!("{perm:b}")
            .chars()
            .zip("rwxrwxrwx".chars())
            .map(|(a, b)| if a == '1' { b } else { '-' })
            .collect();

        Ok(format!("{ftype}{perm}"))
    }

    pub fn fmt_default<T: Display + Default>(x: &Option<T>) -> ::askama::Result<String> {
        if let Some(x) = x {
            Ok(format!("{x}"))
        } else {
            Ok(format!("{}", T::default()))
        }
    }

    /// get json value and convert it to string
    pub fn value_string(json: &Value, key: &str) -> ::askama::Result<String> {
        Ok(json
            .get(key)
            .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_default())
            .unwrap_or_default())
    }

    pub fn value_array_string(json: &Value, key: &str) -> ::askama::Result<Vec<String>> {
        Ok(json
            .get(key)
            .map(|v| {
                v.as_array()
                    .map(|v| {
                        v.iter()
                            .map(|v| v.as_str().unwrap_or_default().to_string())
                            .collect_vec()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default())
    }

    pub fn len<T>(v: &Vec<T>) -> ::askama::Result<usize> {
        Ok(v.len())
    }
    pub fn value_array<'a>(
        json: &'a Value,
        key: &'a str,
    ) -> ::askama::Result<&'a Vec<serde_json::Value>> {
        if let Some(v) = json.get(key) {
            if let Some(v) = v.as_array() {
                Ok(v)
            } else {
                Err(askama::Error::Custom(
                    anyhow::anyhow!("value {v:?} is not array type").into(),
                ))
            }
        } else {
            Err(askama::Error::Custom(
                anyhow::anyhow!("no such key {key} in {json:?}").into(),
            ))
        }
    }

    pub fn value_i64(json: &Value, key: &str) -> ::askama::Result<i64> {
        Ok(json.get(key).map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0))
    }

    pub fn value_i32(json: &Value, key: &str) -> ::askama::Result<i32> {
        Ok(json
            .get(key)
            .map(|v| v.as_i64().unwrap_or(0) as i32)
            .unwrap_or(0))
    }
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
    section: Option<String>,
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

    pub fn get_section(&self) -> &Option<String> {
        &self.section
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
        return not_found!("Repo \"{repo}\" not found.");
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

    let res = repos
        .into_iter()
        .map(|repo| (repo.name.clone(), repo))
        .collect();

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
    pub srcupd: i64,
}
pub async fn db_trees(db: &Ext) -> Result<IndexMap<String, Tree>> {
    let trees: Vec<Tree> = query_as(SQL_GET_TREES).fetch_all(&db.abbs).await?;

    let res = trees
        .into_iter()
        .map(|tree| (tree.name.clone(), tree))
        .collect();

    Ok(res)
}

pub async fn get_tree(tree: &str, db: &Ext) -> Result<Tree> {
    let trees = db_trees(db).await?;
    if let Some(tree) = trees.get(tree) {
        Ok(tree.clone())
    } else {
        return not_found!("Source tree \"{tree}\" not found.");
    }
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

proc_macro_regex::regex!(pub regex_srchost r"^https://(github\.com|bitbucket\.org|gitlab\.com)");
proc_macro_regex::regex!(pub regex_pypi r"^https?://pypi\.(python\.org|io)");
proc_macro_regex::regex!(pub regex_pypisrc r"^https?://pypi\.(python\.org|io)/packages/source/");

const SQL_GET_REPO_COUNT: &str = "
SELECT
  drs.repo name,
  dr.realname realname,
  dr.architecture,
  dr.suite branch,
  dr.date date,
  dr.testing testing,
  dr.category category,
  (drm.name IS NULL) testingonly,
  coalesce(drs.packagecnt, 0) pkgcount,
  coalesce(drs.ghostcnt, 0) ghost,
  coalesce(drs.laggingcnt, 0) lagging,
  coalesce(drs.missingcnt, 0) missing
FROM
  dpkg_repo_stats drs
  LEFT JOIN dpkg_repos dr ON dr.name = drs.repo
  LEFT JOIN (
    SELECT
      drs2.repo repo,
      drs2.packagecnt packagecnt
    FROM
      dpkg_repo_stats drs2
      LEFT JOIN dpkg_repos dr2 ON dr2.name = drs2.repo
    WHERE
      dr2.testing = 0
  ) drs_m ON drs_m.repo = dr.realname
  LEFT JOIN dpkg_repos drm ON drm.realname = dr.realname
  AND drm.testing = 0
ORDER BY
  drs_m.packagecnt DESC,
  dr.realname ASC,
  dr.testing ASC
";

pub const SQL_GET_TREES: &str = "
SELECT
  tree name,
  category,
  url,
  max(date) date,
  count(name) pkgcount,
  sum(ver_compare) srcupd
FROM
  (
    SELECT
      p.name,
      p.tree,
      t.category,
      t.url,
      p.commit_time date,
      (
        CASE
          WHEN vpu.version LIKE (p.version || '%') THEN 0
          ELSE p.version < vpu.version COLLATE vercomp
        END
      ) ver_compare
    FROM
      v_packages p
      INNER JOIN trees t ON t.name = p.tree
      LEFT JOIN piss.v_package_upstream vpu ON vpu.package = p.name
  ) q1
GROUP BY
  tree
ORDER BY
  pkgcount DESC
";

pub const SQL_GET_PACKAGE_TREE: &str = "
SELECT
name,
dpkg.dpkg_version dpkg_version,
group_concat(DISTINCT dpkg.reponame) dpkg_availrepos,
description,
full_version,
ifnull(
  CASE
    WHEN dpkg_version IS NOT null THEN (dpkg_version > full_version COLLATE vercomp) - (dpkg_version < full_version COLLATE vercomp)
    ELSE -1
  END,
  -2
) ver_compare
FROM
v_packages
LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
WHERE
tree = ?
GROUP BY
name
ORDER BY
name
";

pub const SQL_GET_PACKAGE_LAGGING: &str = "
SELECT
  p.name name,
  dpkg.dpkg_version dpkg_version,
  (
    (
      CASE
        WHEN ifnull(pv.epoch, '') = '' THEN ''
        ELSE pv.epoch || ':'
      END
    ) || pv.version || (
      CASE
        WHEN ifnull(pv.release, '') IN ('', '0') THEN ''
        ELSE '-' || pv.release
      END
    )
  ) full_version
FROM
  packages p
  LEFT JOIN package_spec spabhost ON spabhost.package = p.name
  AND spabhost.key = 'ABHOST'
  LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
  LEFT JOIN package_versions pv ON pv.package = p.name
  AND pv.branch = dpkg.branch
WHERE
  dpkg.repo = ?
  AND dpkg_version IS NOT null
  AND (
    dpkg.architecture IS 'noarch'
    OR ? != 'noarch'
  )
  AND (
    (spabhost.value IS 'noarch') = (dpkg.architecture IS 'noarch')
  )
GROUP BY
  name
HAVING
  (
    max(dpkg_version COLLATE vercomp) < full_version COLLATE vercomp
  )
ORDER BY
  name
";

pub const SQL_GET_PACKAGE_LIST: &str = "
SELECT
  p.name,
  p.tree,
  p.tree_category,
  p.branch,
  p.category,
  p.section,
  p.pkg_section,
  p.directory,
  p.description,
  p.version,
  p.full_version,
  p.commit_time,
  p.committer,
  dpkg.dpkg_version dpkg_version,
  group_concat(DISTINCT dpkg.reponame) dpkg_availrepos,
  ifnull(
    CASE
      WHEN dpkg.dpkg_version IS NOT null THEN (
        dpkg.dpkg_version > p.full_version COLLATE vercomp
      ) - (
        dpkg.dpkg_version < p.full_version COLLATE vercomp
      )
      ELSE -1
    END,
    -2
  ) ver_compare
FROM
  v_packages p
  LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = p.name
GROUP BY
  name
ORDER BY
  name
";

pub const SQL_GET_PACKAGE_MISSING: &str = "
SELECT
  v_packages.name name,
  description,
  full_version,
  dpkg_version,
  v_packages.tree_category
FROM
  v_packages
  LEFT JOIN package_spec spabhost ON spabhost.package = v_packages.name
  AND spabhost.key = 'ABHOST'
  LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
  AND dpkg.reponame = ?
WHERE
  full_version IS NOT null
  AND dpkg_version IS null
  AND ((spabhost.value IS 'noarch') = (? IS 'noarch'))
  AND (
  EXISTS(
    SELECT
      1
    FROM
      dpkg_repos
    WHERE
      realname = ?
      AND category = 'bsp'
  ) = (v_packages.tree_category = 'bsp')
)
ORDER BY
  name
";

pub const SQL_GET_PACKAGE_GHOST: &str = "
SELECT
package name,
dpkg_version
FROM
v_dpkg_packages_new
WHERE
repo = ?
AND name NOT IN (
  SELECT
    name
  FROM
    packages
)
GROUP BY
name
";

pub const SQL_GET_PACKAGE_INFO_GHOST: &str = "
SELECT DISTINCT
  package name, '' tree, '' tree_category, '' branch,
  '' category, '' section, '' pkg_section, '' directory,
  '' description, '' version, '' full_version, NULL commit_time, '' committer,
  '' dependency, 0 noarch, NULL fail_arch, NULL srctype, NULL srcurl,
  0 hasrevdep
FROM dpkg_packages WHERE package = ?
";

pub const SQL_SEARCH_PACKAGES_DESC: &str = "
SELECT q.name, q.description, q.desc_highlight, vp.full_version
FROM (
  SELECT
    vp.name, vp.description,
    highlight(fts_packages, 1, '<b>', '</b>') desc_highlight,
    (CASE WHEN vp.name=? THEN 1
     WHEN instr(vp.name, ?)=0 THEN 3 ELSE 2 END) matchcls,
    bm25(fts_packages, 5, 1) ftrank
  FROM packages vp
  INNER JOIN fts_packages fp ON fp.name=vp.name
  WHERE fts_packages MATCH ?
  UNION ALL
  SELECT
    vp.name, vp.description, vp.description desc_highlight,
    2 matchcls, 1.0 ftrank
  FROM v_packages vp
  LEFT JOIN fts_packages fp ON fp.name=vp.name AND fts_packages MATCH ?
  WHERE vp.name LIKE ('%' || ? || '%') AND vp.name!=? AND fp.name IS NULL
) q
INNER JOIN v_packages vp ON vp.name=q.name
ORDER BY q.matchcls, q.ftrank, vp.commit_time DESC, q.name
";

pub const SQL_GET_PACKAGE_SRCUPD: &str = "
SELECT
  vp.name, vp.version, vpu.version upstream_version,
  vpu.updated, vpu.url upstream_url, vpu.tarball upstream_tarball
FROM v_packages vp
INNER JOIN piss.v_package_upstream vpu ON vpu.package=vp.name
WHERE vp.tree=? AND (NOT vpu.version LIKE (vp.version || '%'))
  AND (vp.version < vpu.version COLLATE vercomp)
  AND (? IS NULL OR vp.section = ?)
ORDER BY vp.name
";

pub const SQL_GET_PACKAGE_NEW_LIST: &str = "
SELECT
  name, dpkg.dpkg_version dpkg_version,
  description, full_version, commit_time,
  ifnull(CASE WHEN dpkg_version IS NOT null
   THEN (dpkg_version > full_version COLLATE vercomp) -
   (dpkg_version < full_version COLLATE vercomp)
   ELSE -1 END, -2) ver_compare
FROM v_packages
LEFT JOIN v_dpkg_packages_new dpkg ON dpkg.package = v_packages.name
WHERE full_version IS NOT null
GROUP BY name
ORDER BY commit_time DESC, name ASC
LIMIT ?
";

pub const SQL_GET_PACKAGE_NEW: &str = "
SELECT name, description, full_version, commit_time FROM v_packages
ORDER BY commit_time DESC, name ASC LIMIT 10
";

pub const SQL_GET_PACKAGE_REPO: &str = "
SELECT
  p.name name, p.full_version full_version, dpkg.dpkg_version dpkg_version,
  p.description description
FROM v_packages p
LEFT JOIN package_spec spabhost
  ON spabhost.package = p.name AND spabhost.key = 'ABHOST'
LEFT JOIN v_dpkg_packages_new dpkg
  ON dpkg.package = p.name
WHERE dpkg.repo = ?
  AND ((spabhost.value IS 'noarch') = (dpkg.architecture IS 'noarch'))
ORDER BY p.name
";

pub const SQL_GET_PACKAGE_INFO: &str = "
SELECT
  name, tree, tree_category, branch, category, section, pkg_section, directory,
  description, version, full_version, commit_time, committer,
  dep.dependency dependency,
  (spabhost.value IS 'noarch') noarch, spfailarch.value fail_arch,
  spsrc.key srctype, spsrc.value srcurl,
  EXISTS(SELECT 1 FROM package_dependencies revpd
    WHERE revpd.relationship IN ('PKGDEP', 'BUILDDEP', 'PKGRECOM', 'PKGSUG')
    AND revpd.dependency = v_packages.name) hasrevdep
FROM v_packages
LEFT JOIN (
    SELECT package, group_concat(dependency || '|' || coalesce(relop, '') ||
      coalesce(version, '') || '|' ||
      relationship || '|' || architecture) dependency
    FROM package_dependencies
    GROUP BY package
  ) dep
  ON dep.package = v_packages.name
LEFT JOIN package_spec spabhost
  ON spabhost.package = v_packages.name AND spabhost.key = 'ABHOST'
LEFT JOIN package_spec spfailarch
  ON spfailarch.package = v_packages.name AND spfailarch.key = 'FAIL_ARCH'
LEFT JOIN package_spec spsrc
  ON spsrc.package = v_packages.name
  AND spsrc.key IN ('SRCTBL', 'GITSRC', 'SVNSRC', 'BZRSRC', 'SRCS')
WHERE name = ?
";

pub const SQL_GET_PACKAGE_DPKG: &str = "
SELECT
  version, dp.architecture, repo, dr.realname reponame,
  dr.testing testing, filename, size
FROM dpkg_packages dp
LEFT JOIN dpkg_repos dr ON dr.name=dp.repo
WHERE package = ?
ORDER BY dr.realname ASC, version COLLATE vercomp DESC, testing DESC
";

pub const SQL_GET_PACKAGE_VERSIONS: &str = "
SELECT
  v.branch, ((CASE WHEN ifnull(epoch, '') = '' THEN '' ELSE epoch || ':' END) ||
  version || (CASE WHEN ifnull(release, '') IN ('', '0') THEN '' ELSE '-' ||
  release END)) fullver
FROM package_versions v
INNER JOIN packages p ON p.name=v.package
INNER JOIN tree_branches b ON b.tree=p.tree AND b.branch=v.branch
WHERE v.package = ?
ORDER BY b.priority DESC
";

pub const SQL_GET_PISS_VERSION: &str = "
SELECT version, updated, url FROM piss.v_package_upstream WHERE package=?
";

pub const SQL_GET_PACKAGE_DEB_LOCAL: &str = "
SELECT
  package, version, architecture, repo, maintainer, installed_size,
  filename, size, sha256
FROM dpkg_packages
WHERE package=? AND version=? AND repo=?
";

pub const SQL_GET_PACKAGE_DEB_FILES: &str = r#"
SELECT
  (CASE WHEN path='' THEN '' ELSE '/' || path END) || '/' || "name" filename,
  "size", ftype, perm, uid, gid, uname, gname
FROM pv_package_files
WHERE package=$1 AND version=$2 AND repo=$3 AND ftype!=5
ORDER BY filename
"#;

pub const SQL_GET_PACKAGE_SODEP: &str = "
SELECT depends, name || ver soname
FROM pv_package_sodep
WHERE package=$1 AND version=$2 AND repo=$3
ORDER BY depends, name, ver
";

pub const SQL_ISSUES_STATS: &str = "
SELECT q1.repo, q1.errno, q1.cnt,
  round((q1.cnt::float8/coalesce(q2.total, s.cnt))::numeric, 5)::float8 ratio
FROM (
  SELECT repo, errno, count(DISTINCT package) cnt
  FROM pv_package_issues
  GROUP BY GROUPING SETS ((repo, errno), ())
) q1
LEFT JOIN (
  SELECT repo, count(package) cnt FROM v_packages_new GROUP BY repo
) s ON s.repo=q1.repo
LEFT JOIN (
  SELECT b.name repo, count(DISTINCT p.name) total
  FROM package_versions v
  INNER JOIN packages p ON v.package=p.name
  INNER JOIN tree_branches b ON b.tree=p.tree AND b.branch=v.branch
  GROUP BY GROUPING SETS ((b.name), ())
) q2 ON q2.repo IS NOT DISTINCT FROM q1.repo
";

pub const SQL_ISSUES_RECENT: &str = "
SELECT package, version, array_agg(DISTINCT errno ORDER BY errno) errs
FROM pv_package_issues
WHERE errno!=311
GROUP BY package, version
ORDER BY max(mtime) DESC LIMIT 10
";

pub const SQL_ISSUES_CODE: &str = r#"
SELECT package "name", array_agg(DISTINCT "version") versions,
  array_agg(DISTINCT branch) branches, (array_agg(filename))[1] filename,
  max(filecount) filecount
FROM (
  SELECT package, "version",
    substring(repo from position('/' in repo)+1) branch, max("level") "level",
    (array_agg(filename))[1] filename, count(filename) filecount
  FROM pv_package_issues
  WHERE errno=$1 AND coalesce(repo=$2, TRUE)
  GROUP BY package, version, repo
) q1
GROUP BY package
ORDER BY package
"#;

pub const SQL_ISSUES_PACKAGE: &str = "
SELECT errno, version, repo, filecount, level, filename, detail
FROM (
  SELECT errno, version, repo, level, filename, detail,
    max(rowid) OVER (PARTITION BY errno, version, repo) filecount, rowid
  FROM (
    SELECT errno, version, repo, level, filename, detail,
      count(*) OVER (PARTITION BY errno, version, repo) filecount,
      row_number() OVER (
        PARTITION BY errno, version, repo ORDER BY level, filename) rowid
    FROM pv_package_issues
    WHERE package=$1
  ) q1
) q2
WHERE rowid <= 500
ORDER BY errno, version DESC, repo, level, filename
";

pub const SQL_GET_DEB_LIST_HASARCH: &str = "
SELECT dp.filename, rtrim(
  CASE WHEN dpnew.package IS NULL THEN 'old,' ELSE '' END ||
  CASE WHEN packages.name IS NULL THEN 'outoftree,' ELSE '' END ||
  CASE WHEN (spabhost.value IS 'noarch' AND dpnoarch.package IS NULL)
    THEN 'noarch' ELSE '' END, ',') removereason
FROM dpkg_packages dp
LEFT JOIN (
  SELECT package, max(version COLLATE vercomp) version
  FROM dpkg_packages
  WHERE repo = ?
  GROUP BY package
) dpnew USING (package, version)
LEFT JOIN packages ON packages.name = dp.package
LEFT JOIN package_spec spabhost
  ON spabhost.package = dp.package AND spabhost.key = 'ABHOST'
LEFT JOIN (
  SELECT dp.package, max(dp.version COLLATE vercomp) version
  FROM dpkg_packages dp
  INNER JOIN dpkg_repos dr ON dr.name=dp.repo
  WHERE dr.architecture = 'noarch'
  GROUP BY dp.package
) dpnoarch ON dpnoarch.package=dp.package
AND dpnoarch.version=dpnew.version
WHERE (dpnew.package IS NULL OR packages.name IS NULL
OR (spabhost.value IS 'noarch' AND dpnoarch.package IS NULL))
AND dp.repo=?
UNION ALL
SELECT filename, 'dup' removereason FROM dpkg_package_duplicate WHERE repo=?
ORDER BY filename
";

pub const SQL_GET_DEB_LIST_NOARCH: &str = "
SELECT dp.filename, rtrim(
  CASE WHEN dpnew.package IS NULL THEN 'old,' ELSE '' END ||
  CASE WHEN packages.name IS NULL THEN 'outoftree,' ELSE '' END ||
  CASE WHEN (spabhost.value IS NOT 'noarch' AND dphasarch.package IS NULL)
    THEN 'hasarch' ELSE '' END, ',') removereason
FROM dpkg_packages dp
LEFT JOIN (
  SELECT package, max(version COLLATE vercomp) version
  FROM dpkg_packages
  WHERE repo = ?
  GROUP BY package
) dpnew USING (package, version)
LEFT JOIN packages ON packages.name = dp.package
LEFT JOIN package_spec spabhost
  ON spabhost.package = dp.package AND spabhost.key = 'ABHOST'
LEFT JOIN (
  SELECT dp.package, max(dp.version COLLATE vercomp) version
  FROM dpkg_packages dp
  INNER JOIN dpkg_repos dr ON dr.name=dp.repo
  WHERE dr.architecture != 'noarch'
  GROUP BY dp.package
) dphasarch ON dphasarch.package=dp.package
AND dphasarch.version=dpnew.version
WHERE (dpnew.package IS NULL OR packages.name IS NULL
OR (spabhost.value IS NOT 'noarch' AND dphasarch.package IS NULL))
AND dp.repo=?
UNION ALL
SELECT filename, 'dup' removereason FROM dpkg_package_duplicate WHERE repo=?
ORDER BY filename
";

pub const SQL_GET_PACKAGE_REV_REL: &str = "
SELECT
  package, coalesce(relop, '') || coalesce(version, '') version,
  relationship, architecture
FROM package_dependencies
WHERE dependency = ?
AND relationship IN ('PKGDEP', 'BUILDDEP', 'PKGRECOM', 'PKGSUG')
ORDER BY relationship, package, architecture
";
