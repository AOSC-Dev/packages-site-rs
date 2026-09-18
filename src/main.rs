mod config;
mod db;
pub mod filters;
mod sql;
mod utils;
mod views;

use anyhow::Result;
use axum::{Extension, Router};
use axum_extra::routing::RouterExt;
use clap::Parser;
use config::Config;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use std::sync::Arc;
use tower_http::trace::DefaultOnResponse;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use tracing_subscriber::prelude::*;
use utils::fallback;
use views::*;

const UNIX_SOCKET_PREFIX: &str = "unix:";

#[derive(Parser, Debug)]
#[command(name = "packages-site")]
struct Opt {
    /// specify configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::parse();
    let config = Config::from_file(opt.config)?;

    let subscriber = tracing_subscriber::Registry::default();
    let env_filter = tracing_subscriber::EnvFilter::new(format!(
        "tower_http::trace=trace,packages_site={log},sqlx::query={sqlx_log}",
        log = config.global.log,
        sqlx_log = config.global.sqlx_log
    ));

    let _otel_provider = if let Some(otlp_url) = &config.global.otlp_url {
        // setup otlp
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(otlp_url)
            .build()?;

        let resource = opentelemetry_sdk::Resource::builder()
            .with_service_name("packages-site")
            .build();

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter)
            .build();

        let otlp_tracer = provider.tracer("packages-site");

        // let tracing crate output to opentelemetry
        let tracing_leyer = tracing_opentelemetry::layer().with_tracer(otlp_tracer);
        subscriber
            .with(env_filter)
            .with(tracing_leyer)
            .with(tracing_subscriber::fmt::Layer::default())
            .init();

        Some(provider)
    } else {
        // fallback to stdout
        subscriber
            .with(env_filter)
            .with(tracing_subscriber::fmt::Layer::default())
            .init();

        None
    };

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
        let listener = tokio::net::UnixListener::bind(socket)?;
        axum::serve(listener, service).await?;
    } else {
        let addr = listen.parse::<std::net::SocketAddr>()?;
        info!("package-site is listening on: {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, service).await?;
    }

    Ok(())
}
