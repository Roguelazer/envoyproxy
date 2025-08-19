use axum::{Router, extract::State, routing::get};
use clap::Parser;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::{signal, sync::broadcast};

mod envoy_api;
mod state;

use crate::state::{AppState, Inventory, SystemState};

#[derive(Serialize, Debug)]
struct ResponseBody {
    #[serde(flatten)]
    state: SystemState,
    #[serde(flatten)]
    inventory: Inventory,
}

async fn metrics(State(state): State<Arc<AppState>>) -> axum::response::Json<impl Serialize> {
    let response_body = ResponseBody {
        state: state.system_state.read().await.clone(),
        inventory: state.inventory.read().await.clone(),
    };
    axum::Json(response_body)
}

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

async fn fetch_inventory(state: &AppState, args: &Args) -> anyhow::Result<()> {
    let new_inventory =
        envoy_api::fetch_inventory(&args.envoy_url, &args.envoy_jwt, &state.client).await?;

    let mut guard = state.inventory.write().await;
    *guard = new_inventory;

    Ok(())
}

async fn fetch_once(state: &AppState, args: &Args) -> anyhow::Result<()> {
    let new_state = envoy_api::fetch_state(&args.envoy_url, &args.envoy_jwt, &state.client).await?;

    let mut guard = state.system_state.write().await;
    *guard = new_state;

    Ok(())
}

async fn fetcher(state: Arc<AppState>, args: Args, mut shutdown_rx: broadcast::Receiver<()>) {
    loop {
        if let Err(error) = fetch_once(state.as_ref(), &args).await {
            tracing::error!(?error, "Failed to fetch status!");
        }
        tokio::select! {
            _ = tokio::time::sleep(args.poll_interval()) => {},
            _ = shutdown_rx.recv() => { return }
        }
    }
}

async fn inv_fetcher(state: Arc<AppState>, args: Args, mut shutdown_rx: broadcast::Receiver<()>) {
    loop {
        if let Err(error) = fetch_inventory(state.as_ref(), &args).await {
            tracing::error!(?error, "Failed to fetch inventory!");
        }
        tokio::select! {
            _ = tokio::time::sleep(args.inventory_poll_interval()) => {},
            _ = shutdown_rx.recv() => { return }
        }
    }
}

#[derive(Parser, Debug, Clone)]
struct Args {
    #[arg(short, long, default_value = "3112", env = "PORT")]
    port: u16,
    #[arg(long, default_value = "https://envoy.local", env = "ENVOY_URL")]
    envoy_url: url::Url,
    #[arg(long, env = "ENVOY_JWT")]
    envoy_jwt: String,
    #[arg(
        long,
        default_value = "300",
        help = "Interval to poll the system state, in seconds"
    )]
    poll_interval_secs: u32,
    #[arg(
        long,
        default_value = "21600",
        help = "Interval to collect system inventory, in seconds"
    )]
    inventory_poll_interval_secs: u32,
}

impl Args {
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs as u64)
    }

    fn inventory_poll_interval(&self) -> Duration {
        Duration::from_secs(self.inventory_poll_interval_secs as u64)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt().init();

    let state = Arc::new(AppState::new()?);

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

    tokio::spawn(shutdown_signal(shutdown_tx));
    tokio::spawn(fetcher(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));
    tokio::spawn(inv_fetcher(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));

    let app = Router::new()
        .route("/metrics.json", get(metrics))
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
