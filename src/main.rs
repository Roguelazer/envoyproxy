use axum::{Router, routing::get};
use clap::Parser;
use std::sync::Arc;
use tokio::{signal, sync::broadcast};
use tracing_subscriber::prelude::*;

mod api;
mod args;
mod envoy_api;
mod state;
mod tasks;
mod time_series;

use crate::args::Args;
use crate::state::AppState;
use crate::tasks::BackgroundTask;

async fn shutdown_signal(shutdown_tx: broadcast::Sender<()>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    shutdown_tx.send(()).expect("should be able to shut down");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let state = Arc::new(AppState::new()?);

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

    tracing::trace!("setting up background tasks");

    tokio::spawn(shutdown_signal(shutdown_tx));
    tokio::spawn(tasks::FetchState::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));
    tokio::spawn(tasks::FetchInventory::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));
    tokio::spawn(tasks::MaintainState::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));

    tracing::trace!(port = args.port, "launching server");

    let app = Router::new()
        .route("/metrics.json", get(api::metrics_json))
        .route("/metrics", get(api::metrics_prom))
        .route("/health/ok", get(api::healthcheck))
        .route("/", get(api::root))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("::", args.port))
        .await
        .unwrap();
    tokio::spawn(async {});
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let res = shutdown_rx.recv().await;
            tracing::debug!("shutdown says {:?}", res);
        })
        .await?;
    Ok(())
}
