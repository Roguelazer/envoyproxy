use anyhow::Result;
use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::state::{Inventory, SystemState};

#[derive(Deserialize, Debug)]
pub struct MeterDetails {
    #[serde(rename = "agg_p_mw")]
    pub aggregate_mw: i64,
}

#[derive(Deserialize, Debug)]
pub struct LivestatusMetersResponse {
    pub soc: u32,
    #[serde(with = "ts_seconds")]
    pub last_update: DateTime<Utc>,
    pub pv: MeterDetails,
    pub storage: MeterDetails,
    pub grid: MeterDetails,
    pub load: MeterDetails,
}

#[derive(Deserialize, Debug)]
pub struct LivestatusResponse {
    pub meters: LivestatusMetersResponse,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct EnergyAggregate {
    #[serde(rename = "wattHoursToday")]
    pub watt_hours_today: i64,
    #[serde(rename = "wattHoursSevenDays")]
    pub watt_hours_seven_days: i64,
    #[serde(rename = "wattHoursLifetime")]
    pub watt_hours_lifetime: i64,
    #[serde(rename = "wattsNow")]
    pub watts_now: i64,
}

#[derive(Deserialize, Debug)]
pub struct EnergyProductionResponse {
    #[serde(rename = "eim")]
    pub envoy: EnergyAggregate,
}

#[derive(Deserialize, Debug)]
pub struct EnergyConsumptionResponse {
    #[serde(rename = "eim")]
    pub envoy: EnergyAggregate,
}

#[derive(Deserialize, Debug)]
pub struct EnergyResponse {
    pub production: EnergyProductionResponse,
    pub consumption: EnergyConsumptionResponse,
}

#[derive(Deserialize, Debug)]
pub struct EnchargeDevice {
    pub encharge_capacity: u32,
}

#[derive(Deserialize, Debug)]
pub struct EnpowerDevice {}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum GridState {
    #[serde(rename = "on-grid", alias = "on_grid")]
    OnGrid,
    #[serde(rename = "off-grid", alias = "off_grid")]
    OffGrid,
    #[serde(rename = "multimode-ongrid", alias = "multimode_ongrid")]
    MultiModeOnGrid,
    #[serde(rename = "multimode-offgrid", alias = "multimode_offgrid")]
    MultiModeOffGrid,
}

#[derive(Deserialize, Debug)]
pub struct CollarDevice {
    pub grid_state: GridState,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "devices")]
pub enum InventoryDeviceRow {
    #[serde(rename = "ENCHARGE")]
    Encharge(Vec<EnchargeDevice>),
    #[serde(rename = "ENPOWER")]
    Enpower(Vec<EnpowerDevice>),
    #[serde(rename = "COLLAR")]
    Collar(Vec<CollarDevice>),
}

fn to_dtrait<C: std::fmt::Debug>(o: &C) -> &dyn std::fmt::Debug {
    o
}

impl InventoryDeviceRow {
    pub fn devices(&self) -> Vec<&dyn std::fmt::Debug> {
        match self {
            Self::Encharge(devices) => devices.iter().map(to_dtrait).collect(),
            Self::Enpower(devices) => devices.iter().map(to_dtrait).collect(),
            Self::Collar(devices) => devices.iter().map(to_dtrait).collect(),
        }
    }
}

pub async fn fetch_inventory(
    base_url: &Url,
    envoy_jwt: &str,
    client: &reqwest::Client,
) -> Result<Inventory> {
    let mut inventory_url = base_url.clone();
    inventory_url.set_path("/ivp/ensemble/inventory");
    tracing::trace!(url=?inventory_url, "fetching");
    let inventory_resp: Vec<InventoryDeviceRow> = client
        .get(inventory_url)
        .bearer_auth(envoy_jwt)
        .send()
        .await?
        .json()
        .await?;
    tracing::trace!(response = ?inventory_resp.iter().map(|r| r.devices()).collect::<Vec<_>>(), "fetched inventory");

    let new_inventory = Inventory {
        num_batteries: inventory_resp
            .iter()
            .map(|row| match row {
                InventoryDeviceRow::Encharge(devices) => devices.len(),
                _ => 0,
            })
            .sum(),
        battery_capacity: inventory_resp
            .iter()
            .map(|row| match row {
                InventoryDeviceRow::Encharge(devices) => devices
                    .iter()
                    .map(|device_row| device_row.encharge_capacity)
                    .sum(),
                _ => 0,
            })
            .sum(),
        grid_state: inventory_resp.iter().find_map(|row| match row {
            InventoryDeviceRow::Collar(devices) => devices.first().map(|s| s.grid_state),
            _ => None,
        }),
    };
    Ok(new_inventory)
}

pub async fn fetch_state(
    base_url: &Url,
    envoy_jwt: &str,
    client: &reqwest::Client,
) -> Result<SystemState> {
    let mut status_url = base_url.clone();
    status_url.set_path("/ivp/livedata/status");
    tracing::trace!(url = ?status_url, "fetching");
    let status_resp: LivestatusResponse = client
        .get(status_url)
        .bearer_auth(envoy_jwt)
        .send()
        .await?
        .json()
        .await?;
    tracing::trace!(response = ?status_resp, "fetched status");

    let mut energy_url = base_url.clone();
    energy_url.set_path("/ivp/pdm/energy");
    tracing::trace!(url=?energy_url, "fetching");
    let energy_resp: EnergyResponse = client
        .get(energy_url)
        .bearer_auth(envoy_jwt)
        .send()
        .await?
        .json()
        .await?;
    tracing::trace!(response = ?energy_resp, "fetched energy");

    let new_state = SystemState {
        last_update: Some(status_resp.meters.last_update),
        production_mwh_today: energy_resp.production.envoy.watt_hours_today * 1000,
        consumption_mwh_today: energy_resp.consumption.envoy.watt_hours_today * 1000,
        pv_mw: status_resp.meters.pv.aggregate_mw,
        grid_mw: status_resp.meters.grid.aggregate_mw,
        storage_mw: status_resp.meters.storage.aggregate_mw,
        load_mw: status_resp.meters.load.aggregate_mw,
        battery_soc: status_resp.meters.soc,
    };
    Ok(new_state)
}
