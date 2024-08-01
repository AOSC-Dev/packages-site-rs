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
use hyper::Server;
use hyperlocal::UnixServerExt;
use opentelemetry_otlp::WithExportConfig;
use std::sync::Arc;
use structopt::StructOpt;
use tower_http::trace::DefaultOnResponse;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use tracing_subscriber::prelude::*;
use utils::fallback;
use views::*;

const UNIX_SOCKET_PREFIX: &str = "unix:";

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

    let subscriber = tracing_subscriber::Registry::default();
    let env_filter = tracing_subscriber::EnvFilter::new(format!(
        "tower_http::trace=trace,packages_site={log},sqlx::query={sqlx_log}",
        log = config.global.log,
        sqlx_log = config.global.sqlx_log
    ));
    if let Some(otlp_url) = &config.global.otlp_url {
        // setup otlp
        let exporter = opentelemetry_otlp::new_exporter().http().with_endpoint(otlp_url);
        let otlp_tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_trace_config(
                opentelemetry_sdk::trace::config().with_resource(opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", "packages-site"),
                ])),
            )
            .with_exporter(exporter)
            .install_batch(opentelemetry_sdk::runtime::Tokio)?;

        // let tracing crate output to opentelemetry
        let tracing_leyer = tracing_opentelemetry::layer().with_tracer(otlp_tracer);
        subscriber
            .with(env_filter)
            .with(tracing_leyer)
            .with(tracing_subscriber::fmt::Layer::default())
            .init();
    } else {
        // fallback to stdout
        subscriber
            .with(env_filter)
            .with(tracing_subscriber::fmt::Layer::default())
            .init();
    }

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
        .typed_get(cleanmirror)
        .typed_get(revdep)
        .typed_get(license)
        .fallback(fallback)
        .layer(
            TraceLayer::new_for_http()
                .on_request(())
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(Extension(db));

    let service = app.into_make_service();

    let listen = &config.global.listen;
    if let Some(socket) = listen.strip_prefix(UNIX_SOCKET_PREFIX) {
        info!("package-site is listening on unix socket: {}", socket);
        Server::bind_unix(socket)?.serve(service).await?;
    } else {
        let addr = listen.parse()?;
        info!("package-site is listening on: {}", addr);
        Server::bind(&addr).serve(service).await?;
    }

    Ok(())
}
