use axum::{Router, extract::State, routing::get};
use clap::Parser;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::{signal, sync::broadcast};

mod envoy_api;
mod state;
mod time_series;

use crate::state::{AppState, Inventory, SystemState};

#[derive(Serialize, Debug)]
struct ResponseBody {
    #[serde(flatten)]
    state: SystemState,
    #[serde(flatten)]
    inventory: Inventory,
    history: state::HistoryResponse,
}

async fn metrics(State(state): State<Arc<AppState>>) -> axum::response::Json<impl Serialize> {
    let response_body = ResponseBody {
        state: state.system_state.read().await.clone(),
        inventory: state.inventory.read().await.clone(),
        history: state.history().await,
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

trait BackgroundTask {
    const LABEL: &'static str;

    async fn run(state: &AppState, args: &Args) -> anyhow::Result<()>;

    async fn start(
        state: Arc<AppState>,
        args: Args,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> anyhow::Result<()> {
        let interval = Self::interval(&args);
        loop {
            tracing::debug!(label = Self::LABEL, "invoking background task");
            if let Err(error) = Self::run(state.as_ref(), &args).await {
                tracing::error!(?error, "{}", Self::LABEL);
            }
            tokio::select! {
                    _ = tokio::time::sleep(interval) => {},
                    _ = shutdown_rx.recv() => { return Ok(()) }
            }
        }
    }

    fn interval(args: &Args) -> Duration;
}

struct FetchInventory {}

impl BackgroundTask for FetchInventory {
    const LABEL: &'static str = "fetch inventory";

    async fn run(state: &AppState, args: &Args) -> anyhow::Result<()> {
        let new_inventory =
            envoy_api::fetch_inventory(&args.envoy_url, &args.envoy_jwt, &state.client).await?;

        let mut guard = state.inventory.write().await;
        *guard = new_inventory;

        Ok(())
    }

    fn interval(args: &Args) -> Duration {
        args.inventory_poll_interval()
    }
}

struct FetchState {}

impl BackgroundTask for FetchState {
    const LABEL: &'static str = "fetch state";

    async fn run(state: &AppState, args: &Args) -> anyhow::Result<()> {
        let new_state =
            envoy_api::fetch_state(&args.envoy_url, &args.envoy_jwt, &state.client).await?;

        state.update_state(new_state).await;

        Ok(())
    }

    fn interval(args: &Args) -> Duration {
        args.poll_interval()
    }
}

struct MaintainState {}

impl BackgroundTask for MaintainState {
    const LABEL: &'static str = "maintain state";

    async fn run(state: &AppState, _args: &Args) -> anyhow::Result<()> {
        state.maintain().await;
        Ok(())
    }

    fn interval(_args: &Args) -> Duration {
        Duration::from_secs(1800)
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

    tracing::trace!("setting up background tasks");

    tokio::spawn(shutdown_signal(shutdown_tx));
    tokio::spawn(FetchState::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));
    tokio::spawn(FetchInventory::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));
    tokio::spawn(MaintainState::start(
        Arc::clone(&state),
        args.clone(),
        shutdown_rx.resubscribe(),
    ));

    tracing::trace!("launching server");

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
