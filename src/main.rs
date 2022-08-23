mod config;
mod db;
mod filters;
mod sql;
mod utils;
mod views;

use axum::{handler::Handler, routing::get_service, Extension, Router};
use axum_extra::routing::RouterExt;
use config::Config;
use std::sync::Arc;
use tower_http::{services::ServeDir, trace::TraceLayer};
use utils::fallback;
use utils::Error;
use views::*;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("tower_http::trace=trace,packages_site=debug,sqlx::query=warn")
        .init();

    let config = Config::from_file("config.toml").unwrap();
    let db = Arc::new(db::Db::open(&config).await.unwrap());

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
        .nest(
            "/data",
            get_service(ServeDir::new(&config.data)).handle_error(|err| async { Error::from(err) }),
        )
        .fallback(fallback.into_service())
        .layer(TraceLayer::new_for_http())
        .layer(Extension(db));

    axum::Server::bind(&config.listen.parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
