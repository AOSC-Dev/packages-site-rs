use crate::db::{get_page, Page};
use crate::filters;
use crate::sql::*;
use crate::utils::*;
use askama::Template;
use axum::response::IntoResponse;
use itertools::Itertools;
use serde::Serialize;
use sqlx::FromRow;

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

    let (page, packages) = get_page!(SQL_GET_PACKAGE_TREE, Package, q.get_page(), &db.abbs, &tree).await?;

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

    let ctx = Template { page, tree, packages };

    let ctx_tsv = TemplateTsv { packages };

    render(ctx, Some(ctx_tsv), &q)
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
