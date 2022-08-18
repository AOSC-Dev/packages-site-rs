use crate::db::{get_page, Page};
use crate::filters;
use crate::sql::*;
use crate::utils::*;
use askama::Template;
use axum::response::{IntoResponse, Redirect};
use serde::Serialize;
use sqlx::FromRow;

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

    let qesc = format!("\"{q}\"");

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

            let name_highlight = html_escape::encode_safe(&pkg.name).replace(q, &format!("<b>{q}</b>"));

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
