use crate::filters;
use crate::sql::*;
use crate::utils::*;
use askama::Template;
use axum::response::IntoResponse;
use itertools::Itertools;
use serde::Serialize;
use sqlx::{query_as, FromRow};

typed_path!("/", Index);
pub async fn index(_: Index, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        name: String,
        description: String,
        full_version: String,
        commit_time: time::OffsetDateTime,
        ver_compare: i32,
        status: i32,
    }

    #[derive(Template, Serialize)]
    #[template(path = "index.html")]
    struct Template {
        total: i64,
        repo_categories: Vec<(String, Vec<Repo>)>,
        updates: Vec<Package>,
    }

    let source_trees = db_trees(&db).await?;
    let repos = db_repos(&db).await?;

    let repo_categories = REPO_CAT
        .iter()
        .map(|(category_capital, category)| {
            let repos = repos
                .iter()
                .filter_map(|(_name, repo)| (&repo.category == category_capital).then_some(repo.clone()))
                .collect();

            (category.to_string(), repos)
        })
        .collect_vec();

    let total: i64 = source_trees.iter().map(|(_name, repo)| repo.pkgcount).sum();
    let updates = query_as(SQL_GET_PACKAGE_NEW).fetch_all(&db.meta).await?;

    let ctx = Template {
        total,
        repo_categories,
        updates,
    };

    render::<_, Template>(ctx, None, &q)
}

typed_path!("/updates", Updates);
pub async fn updates(_: Updates, q: Query, db: Ext) -> Result<impl IntoResponse> {
    #[derive(FromRow, Serialize)]
    struct Package {
        name: String,
        dpkg_version: String,
        description: String,
        full_version: String,
        commit_time: time::OffsetDateTime,
        ver_compare: i32,
        status: i32,
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

    let packages: &Vec<Package> = &query_as(SQL_GET_PACKAGE_NEW_LIST).bind(100).fetch_all(&db.meta).await?;

    if packages.is_empty() {
        not_found!("There's no updates.");
    }

    let ctx = Template { packages };
    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
}

typed_path!("/license", License);
pub async fn license(_: License) -> Result<impl IntoResponse> {
    #[derive(Template)]
    #[template(path = "license.html")]
    struct Template;

    Ok(into_response(&Template {}, None))
}
