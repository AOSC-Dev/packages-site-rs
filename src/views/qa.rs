use crate::db::{Page, Paginator};
use crate::filters;
use crate::sql::*;
use crate::utils::*;
use anyhow::Context;
use askama::Template;
use axum::response::{IntoResponse, Redirect, Response};
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use serde::Serialize;
use sqlx::{query, query_as, FromRow};
use std::collections::{HashMap, HashSet};

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

    let (ref pkgs, page): (Vec<Package>, _) = query_as(SQL_ISSUES_CODE)
        .bind(&code)
        .bind(&repo)
        .fetch_page(&db.pg, q.get_page())
        .await?;

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

    let issues: Vec<PkgIssue> = query_as(SQL_ISSUES_PACKAGE).bind(name).fetch_all(&db.pg).await?;

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
                        summary: IndexSet<String>,
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
                    for PkgIssue { filename, detail, .. } in issues {
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
                        .collect();

                    serde_json::to_value(Custom { summary, files_bypkg }).unwrap_or_default()
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
