use crate::db::{Page, Paginator};
use crate::filters;
use crate::sql::*;
use crate::utils::*;
use askama::Template;
use axum::response::IntoResponse;
use itertools::Itertools;
use serde::Serialize;
use sqlx::{query_as, FromRow};

typed_path!("/repo/*repo", RouteRepo, repo);
pub async fn repo(RouteRepo { repo }: RouteRepo, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow)]
    struct Package {
        name: String,
        full_version: String,
        dpkg_version: String,
        description: String,
        status: i32,
    }

    #[derive(Serialize)]
    struct PackageTemplate {
        ver_compare: i32,
        name: String,
        dpkg_version: String,
        description: String,
        status: i32,
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

    let repo = strip_prefix(&repo);
    get_repo(repo, &db).await?;

    let (packages, page): (Vec<Package>, _) = query_as(SQL_GET_PACKAGE_REPO)
        .bind(repo)
        .fetch_page(&db.meta, q.get_page())
        .await?;

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
                status: pkg.status,
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

    let repo = strip_prefix(&repo);
    let architecture = get_repo(repo, &db).await?.architecture;

    let (ref packages, page): (Vec<Package>, _) = query_as(SQL_GET_PACKAGE_LAGGING)
        .bind(repo)
        .bind(architecture)
        .fetch_page(&db.meta, q.get_page())
        .await?;

    if packages.is_empty() {
        not_found!("There's no lagging packages.");
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

    let repo = strip_prefix(&repo);
    let repo = get_repo(repo, &db).await?;

    let (ref packages, page): (Vec<Package>, _) = query_as(SQL_GET_PACKAGE_MISSING)
        .bind(&repo.realname)
        .bind(&repo.architecture)
        .bind(&repo.realname)
        .fetch_page(&db.meta, q.get_page())
        .await?;

    if packages.is_empty() {
        not_found!("There's no missing packages.");
    }

    let ctx = Template {
        page,
        repo: repo.name.to_string(),
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

    let repo = strip_prefix(&repo);
    get_repo(repo, &db).await?;

    let (ref packages, page): (Vec<Package>, _) = query_as(SQL_GET_PACKAGE_GHOST)
        .bind(repo)
        .fetch_page(&db.meta, q.get_page())
        .await?;

    if packages.is_empty() {
        not_found!("There's no ghost packages.");
    }

    let ctx = Template {
        packages,
        repo: repo.to_string(),
        page,
    };
    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}
