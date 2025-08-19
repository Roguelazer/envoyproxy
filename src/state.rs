use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::time_series::{TimeSeriesRow, TimeSeriesSummary};

#[derive(Serialize, Debug, Default, Clone)]
pub struct SystemState {
    pub last_update: Option<DateTime<Utc>>,
    pub battery_soc: u32,
    pub pv_mw: i64,
    pub storage_mw: i64,
    pub grid_mw: i64,
    pub load_mw: i64,
    pub production_mwh_today: i64,
    pub consumption_mwh_today: i64,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct Inventory {
    pub battery_capacity: u32,
    pub num_batteries: usize,
}

#[derive(Debug, Default)]
pub struct TimeSeriesData {
    pv_mw: TimeSeriesRow,
    storage_mw: TimeSeriesRow,
    load_mw: TimeSeriesRow,
    grid_mw: TimeSeriesRow,
}

#[derive(Serialize, Debug)]
pub struct HistoryResponse {
    pv_mw: TimeSeriesSummary,
    grid_mw: TimeSeriesSummary,
    load_mw: TimeSeriesSummary,
    storage_mw: TimeSeriesSummary,
}

pub struct AppState {
    pub client: reqwest::Client,
    pub system_state: RwLock<SystemState>,
    pub inventory: RwLock<Inventory>,
    pub time_series: RwLock<TimeSeriesData>,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .use_native_tls()
            // envoy makes up totally bogus certs
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        let system_state = RwLock::new(SystemState::default());
        let inventory = RwLock::new(Inventory::default());
        let time_series = RwLock::new(TimeSeriesData::default());
        Ok(Self {
            client,
            system_state,
            inventory,
            time_series,
        })
    }

    pub async fn history(&self) -> HistoryResponse {
        let ts = self.time_series.read().await;
        HistoryResponse {
            pv_mw: ts.pv_mw.summary(),
            grid_mw: ts.grid_mw.summary(),
            load_mw: ts.load_mw.summary(),
            storage_mw: ts.storage_mw.summary(),
        }
    }

    pub async fn update_state(&self, new_state: SystemState) {
        let Some(dt) = new_state.last_update else {
            return;
        };
        let mut time_series_guard = self.time_series.write().await;
        time_series_guard.pv_mw.append(dt, new_state.pv_mw);
        time_series_guard.grid_mw.append(dt, new_state.grid_mw);
        time_series_guard.load_mw.append(dt, new_state.load_mw);
        time_series_guard
            .storage_mw
            .append(dt, new_state.storage_mw);
        drop(time_series_guard);

        let mut state_guard = self.system_state.write().await;
        *state_guard = new_state;
        drop(state_guard);
    }

    pub async fn maintain(&self) {
        let mut time_series_guard = self.time_series.write().await;
        time_series_guard.pv_mw.maintain();
        time_series_guard.grid_mw.maintain();
        time_series_guard.load_mw.maintain();
        time_series_guard.storage_mw.maintain();
    }
}
