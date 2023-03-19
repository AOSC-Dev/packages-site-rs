mod config;
mod db;
mod filters;
mod sql;
mod utils;
mod views;

use anyhow::Result;
use axum::{Extension, Router};
use axum_extra::routing::RouterExt;
use config::Config;
use tower_http::services::ServeFile;
use std::sync::Arc;
use structopt::StructOpt;
use tower_http::trace::DefaultOnResponse;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use utils::fallback;
use views::*;

#[derive(StructOpt, Debug)]
#[structopt(name = "packages-site")]
struct Opt {
    /// specify configuration file
    #[structopt(short, long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();
    let config = Config::from_file(opt.config)?;
    tracing_subscriber::fmt()
        .with_env_filter(format!(
            "tower_http::trace=trace,packages_site={log},sqlx::query={sqlx_log}",
            log = config.global.log,
            sqlx_log = config.global.sqlx_log
        ))
        .init();

    let db = Arc::new(db::Db::open(&config).await?);

    let app = Router::new()
        .typed_get(static_files)
        .typed_get(changelog)
        .typed_get(index)
        .typed_get(pkgtrie)
        .typed_get(pkglist)
        .typed_get(lagging)
        .typed_get(missing)
        .typed_get(ghost)
        .typed_get(search)
        .typed_get(updates)
        .typed_get(repo)
        .typed_get(packages)
        .typed_get(files)
        .typed_get(qa)
        .typed_get(qa_index)
        .typed_get(qa_code)
        .typed_get(qa_repo)
        .typed_get(qa_package)
        .typed_get(cleanmirror)
        .typed_get(revdep)
        .typed_get(license)
        .nest_service("/data", ServeFile::new(config.db.abbs))
        .fallback(fallback)
        .layer(
            TraceLayer::new_for_http()
                .on_request(())
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(Extension(db));

    let url = &config.global.listen.parse()?;

    info!("package-site is running at: http://{}", url);

    axum::Server::bind(url).serve(app.into_make_service()).await?;

    Ok(())
}
