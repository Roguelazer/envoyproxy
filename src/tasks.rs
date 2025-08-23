use tokio::sync::broadcast;

use std::sync::Arc;
use std::time::Duration;

use crate::args::Args;
use crate::envoy_api;
use crate::state::AppState;

pub trait BackgroundTask {
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

pub struct FetchInventory {}

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

pub struct FetchState {}

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

pub struct MaintainState {}

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
