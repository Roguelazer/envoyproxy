use axum::{Router, extract::State, routing::get};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::{signal, sync::broadcast};

struct AppState {
    client: reqwest::Client,
    system_state: RwLock<SystemState>,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .use_native_tls()
            // envoy makes up totally bogus certs
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        let system_state = RwLock::new(SystemState::default());
        Ok(Self {
            client,
            system_state,
        })
    }
}

async fn metrics(State(state): State<Arc<AppState>>) -> axum::response::Json<SystemState> {
    axum::Json(state.system_state.read().await.clone())
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

#[derive(Serialize, Debug, Default, Clone)]
struct SystemState {
    last_update: DateTime<Utc>,
    battery_soc: u32,
    pv_mw: i64,
    storage_mw: i64,
    grid_mw: i64,
    load_mw: i64,
    production_mwh_today: i64,
    consumption_mwh_today: i64,
}

#[derive(Deserialize, Debug)]
struct MeterDetails {
    #[serde(rename = "agg_p_mw")]
    aggregate_mw: i64,
}

#[derive(Deserialize, Debug)]
struct LivestatusMetersResponse {
    soc: u32,
    #[serde(with = "ts_seconds")]
    last_update: DateTime<Utc>,
    pv: MeterDetails,
    storage: MeterDetails,
    grid: MeterDetails,
    load: MeterDetails,
}

#[derive(Deserialize, Debug)]
struct LivestatusResponse {
    meters: LivestatusMetersResponse,
}

#[derive(Deserialize, Debug)]
struct EnergyAggregate {
    #[serde(rename = "wattHoursToday")]
    watt_hours_today: i64,
    #[serde(rename = "wattHoursSevenDays")]
    watt_hours_seven_days: i64,
    #[serde(rename = "wattHoursLifetime")]
    watt_hours_lifetime: i64,
    #[serde(rename = "wattsNow")]
    watts_now: i64,
}

#[derive(Deserialize, Debug)]
struct EnergyProductionResponse {
    #[serde(rename = "eim")]
    envoy: EnergyAggregate,
}

#[derive(Deserialize, Debug)]
struct EnergyConsumptionResponse {
    #[serde(rename = "eim")]
    envoy: EnergyAggregate,
}

#[derive(Deserialize, Debug)]
struct EnergyResponse {
    production: EnergyProductionResponse,
    consumption: EnergyConsumptionResponse,
}

async fn fetch_once(state: &AppState, args: &Args) -> anyhow::Result<()> {
    let mut status_url = args.envoy_host.clone();
    status_url.set_path("/ivp/livedata/status");
    tracing::trace!(url = ?status_url, "fetching");
    let status_resp: LivestatusResponse = state
        .client
        .get(status_url)
        .bearer_auth(&args.envoy_jwt)
        .send()
        .await?
        .json()
        .await?;
    tracing::trace!(response = ?status_resp, "fetched status");
    let mut energy_url = args.envoy_host.clone();
    energy_url.set_path("/ivp/pdm/energy");
    tracing::trace!(url=?energy_url, "fetching");
    let energy_resp: EnergyResponse = state
        .client
        .get(energy_url)
        .bearer_auth(&args.envoy_jwt)
        .send()
        .await?
        .json()
        .await?;
    tracing::trace!(response = ?energy_resp, "fetched energy");

    let new_state = SystemState {
        last_update: status_resp.meters.last_update,
        production_mwh_today: energy_resp.production.envoy.watt_hours_today * 1000,
        consumption_mwh_today: energy_resp.consumption.envoy.watt_hours_today * 1000,
        pv_mw: status_resp.meters.pv.aggregate_mw,
        grid_mw: status_resp.meters.grid.aggregate_mw,
        storage_mw: status_resp.meters.storage.aggregate_mw,
        load_mw: status_resp.meters.load.aggregate_mw,
        battery_soc: status_resp.meters.soc,
    };

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

#[derive(Parser, Debug, Clone)]
struct Args {
    #[arg(short, long, default_value = "3112", env = "PORT")]
    port: u16,
    #[arg(long, default_value = "https://envoy.local", env = "ENVOY_HOST")]
    envoy_host: url::Url,
    #[arg(long, env = "ENVOY_JWT")]
    envoy_jwt: String,
    #[arg(long, default_value = "120")]
    poll_interval_secs: u32,
}

impl Args {
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs as u64)
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
