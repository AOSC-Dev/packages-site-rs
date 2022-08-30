use crate::filters;
use crate::sql::*;
use crate::utils::*;
use abbs_meta_tree::package::FailArch;
use anyhow::anyhow;
use askama::Template;
use axum::response::IntoResponse;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Serialize;
use sqlx::{query, query_as, FromRow};
use std::collections::{HashMap, HashSet};
use std::iter::repeat;

typed_path!("/packages/:name", RoutePackage, name);
pub async fn packages(RoutePackage { name }: RoutePackage, q: Query, db: Ext) -> Result<impl IntoResponse> {
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
        spec_path: String,
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
        testing: bool,
        version: String,
        url: String,
        branch: String,
    }

    #[derive(FromRow, Debug)]
    struct PackageVersion {
        branch: String,
        fullver: String,
    }

    #[derive(FromRow, Serialize, Debug)]
    struct PackageError {
        message: String,
        path: String,
        tree: String,
        branch: String,
        col: Option<u32>,
        line: Option<u32>,
    }

    #[derive(FromRow)]
    struct PackageTesting {
        pub version: String,
        pub tree: String,
        pub branch: String,
        pub spec_path: String,
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
    let errors: Vec<PackageError> = query_as(SQL_GET_PACKAGE_ERRORS).bind(&name).fetch_all(&db.abbs).await?;

    // Generate version matrix

    let dpkgs: Vec<DpkgPackage> = query_as(SQL_GET_PACKAGE_DPKG).bind(&name).fetch_all(&db.abbs).await?;

    // 1.1 process package testing
    let testing_vers: Vec<PackageTesting> = query_as(SQL_GET_PACKAGE_TESTING)
        .bind(&name)
        .fetch_all(&db.abbs)
        .await?;
    let mut testing_ver_count = HashMap::new();
    testing_vers.iter().for_each(|PackageTesting { version, .. }| {
        testing_ver_count
            .entry(version.clone())
            .and_modify(|e| *e += 1)
            .or_insert(1);
    });
    let testing_vers: HashMap<_, _> = testing_vers
        .into_iter()
        .map(|testing| {
            let version = if testing_ver_count[&testing.version] >= 2 {
                let version = &testing.version;
                let branch = testing
                    .branch
                    .strip_prefix("origin/")
                    .unwrap_or(testing.branch.as_str());
                format!("{version}-{branch}")
            } else {
                testing.version.clone()
            };
            (version, testing)
        })
        .collect();

    // 1.2 collect vers list from dpkgs and package_testing, sorted by desc
    let mut vers: HashSet<_> = dpkgs.iter().map(|x| x.version.clone()).collect();
    let fullver = &pkg.full_version;
    if pkgintree & !fullver.is_empty() & !vers.contains(fullver) {
        vers.insert(fullver.clone());
    }
    vers.extend(testing_vers.keys().cloned());

    let vers = vers
        .into_iter()
        .sorted_by(|a, b| deb_version::compare_versions(a, b).reverse())
        .collect_vec();

    // 2.1 generate src_vers from SQL query
    let src_vers: Vec<PackageVersion> = query_as(SQL_GET_PACKAGE_VERSIONS)
        .bind(&name)
        .fetch_all(&db.abbs)
        .await?;
    let src_vers: HashMap<_, _> = src_vers.into_iter().map(|v| (v.fullver, v.branch)).collect();

    // 3. generate versions list
    let versions = vers
        .iter()
        .map(|version| {
            let src = src_vers.get(version);
            let testing = testing_vers.get(version);
            match (testing, src) {
                (
                    Some(PackageTesting {
                        tree,
                        branch,
                        spec_path,
                        ..
                    }),
                    _,
                ) => {
                    let branch = branch.strip_prefix("origin/").unwrap_or(branch.as_str());
                    let url = format!("https://github.com/AOSC-Dev/{tree}/tree/{branch}/{spec_path}");
                    Version {
                        version: version.clone(),
                        url,
                        branch: branch.into(),
                        testing: true,
                    }
                }
                (None, Some(src_branch)) => {
                    let url = format!(
                        "https://github.com/AOSC-Dev/{tree}/tree/{src_branch}/{spec_path}",
                        tree = &pkg.tree,
                        spec_path = &pkg.spec_path
                    );

                    Version {
                        version: version.clone(),
                        url,
                        branch: src_branch.into(),
                        testing: false,
                    }
                }
                (None, None) => Version {
                    version: version.clone(),
                    url: "".into(),
                    branch: "".into(),
                    testing: false,
                },
            }
        })
        .collect_vec();

    // 4. generate reponames list, sorted by asc
    let reponames = if pkg.noarch {
        vec!["noarch".into()]
    } else if !pkg.tree_category.is_empty() & !pkg.fail_arch.is_empty() {
        let fail_arch = FailArch::from(&pkg.fail_arch).map_err(|_| anyhow!("invalid FAIL_ARCH format"))?;
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

                    dpkg.map(|dpkg| {
                        let (size, testing, repo) = (dpkg.size, dpkg.testing, dpkg.repo.clone());
                        DpkgMeta {
                            hasmeta: true,
                            version: ver.version.clone(),
                            testing,
                            repo,
                            size,
                        }
                    })
                    .unwrap_or_default()
                })
                .collect_vec();

            MatrixRow {
                repo: repo.clone(),
                meta,
            }
        })
        .collect_vec();

    // guess upstream url
    let (srcurl_base, srcurl, srctype) = match Src::parse(&pkg.srctype, &pkg.srcurl) {
        Some(Src { srcurl, srctype }) => {
            let srcurl_base = match srcurl {
                _ if regex_srchost(&srcurl) => srcurl.split('/').take(5).collect_vec().join("/"),
                _ if regex_pypi(&srcurl) => {
                    let pypiname = if regex_pypisrc(&srcurl) {
                        srcurl.split('/').nth_back(2)
                    } else {
                        srcurl
                            .split('/')
                            .nth_back(1)
                            .and_then(|s| s.rsplit_once('-').map(|x| x.0))
                    };

                    pypiname
                        .map(|pypiname| format!("https://pypi.python.org/pypi/{pypiname}/"))
                        .unwrap_or_default()
                }
                _ => match srctype {
                    SrcType::Tarball => srcurl.split_once('/').map(|x| x.0.to_string()).unwrap_or_default(),
                    SrcType::Git => srcurl
                        .strip_prefix("git://")
                        .map(|stripped| format!("http://{stripped}"))
                        .unwrap_or_else(|| srcurl.clone()),
                    _ => "".into(),
                },
            };
            (srcurl_base, srcurl, srctype.to_string())
        }
        None => ("".into(), "".into(), "".into()),
    };

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
    };

    render::<_, Template>(ctx, None, &q)
}

typed_path!("/changelog/:name", Changelog, name);
pub async fn changelog(Changelog { name }: Changelog, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(Debug, FromRow, Serialize)]
    struct Change {
        pub package: String,
        pub githash: String,
        pub version: String,
        pub tree: String,
        pub branch: String,
        pub urgency: String,
        pub message: String,
        pub maintainer_name: String,
        pub maintainer_email: String,
        pub timestamp: i64,
    }

    let changes: Vec<Change> = query_as(SQL_GET_PACKAGE_CHANGELOG)
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
                Some(TemplateRevDep { description, deps: res })
            } else {
                None
            }
        })
        .collect_vec();

    let sobreaks: Vec<Sobreak> = query_as("SELECT dep_package, deplist FROM v_so_breaks_dep WHERE package=$1")
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
        architecture: String,
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
