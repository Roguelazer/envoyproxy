use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Serialize, Debug, Default, Clone)]
pub struct SystemState {
    pub last_update: DateTime<Utc>,
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

pub struct AppState {
    pub client: reqwest::Client,
    pub system_state: RwLock<SystemState>,
    pub inventory: RwLock<Inventory>,
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
        Ok(Self {
            client,
            system_state,
            inventory,
        })
    }
}
