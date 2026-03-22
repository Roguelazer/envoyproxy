use anyhow::Context;
use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;

use crate::envoy_api::GridState;
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
    pub grid_state: Option<GridState>,
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum HistoryKind {
    Pv = 0,
    Grid = 1,
    Load = 2,
    Storage = 3,
}

impl TryFrom<u8> for HistoryKind {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Pv),
            1 => Ok(Self::Grid),
            2 => Ok(Self::Load),
            3 => Ok(Self::Storage),
            _ => anyhow::bail!("invalid discriminant"),
        }
    }
}

#[derive(Debug, Default)]
pub struct TimeSeriesData {
    pv_mw: TimeSeriesRow,
    storage_mw: TimeSeriesRow,
    load_mw: TimeSeriesRow,
    grid_mw: TimeSeriesRow,
}

impl TimeSeriesData {
    #[tracing::instrument(skip_all)]
    fn load_from_db(&mut self, db: &mut rusqlite::Connection) -> anyhow::Result<()> {
        tracing::debug!("beginning load of historical data from database");
        let mut stmt =
            db.prepare("SELECT kind, timestamp, value FROM history ORDER BY 1, 2 ASC")?;
        let mut loaded = 0;
        let rows = stmt
            .query_map([], |row| -> rusqlite::Result<(u8, i64, i64)> {
                let history_kind: u8 = row.get(0)?;
                let timestamp: i64 = row.get(1)?;
                let value = row.get(2)?;
                Ok((history_kind, timestamp, value))
            })?
            .map(|r| {
                r.context("error reading from sqlite").and_then(
                    |(history_kind, timestamp, value)| {
                        let history_kind = HistoryKind::try_from(history_kind)?;
                        let timestamp = DateTime::<Utc>::from_timestamp(timestamp, 0)
                            .ok_or_else(|| anyhow::anyhow!("invalid timestamp"))?;
                        Ok((history_kind, timestamp, value))
                    },
                )
            });
        for row in rows {
            let (history_kind, timestamp, value) = row?;
            loaded += 1;
            match history_kind {
                HistoryKind::Pv => self.pv_mw.append(timestamp, value),
                HistoryKind::Grid => self.grid_mw.append(timestamp, value),
                HistoryKind::Load => self.load_mw.append(timestamp, value),
                HistoryKind::Storage => self.storage_mw.append(timestamp, value),
            }
        }
        tracing::debug!(?loaded, "finished load of historical data from database");
        Ok(())
    }
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
    pub db: Arc<Mutex<rusqlite::Connection>>,
}

impl AppState {
    pub fn new<P: AsRef<Path>>(store_path: P) -> anyhow::Result<Self> {
        let store_path = store_path.as_ref();
        tracing::debug!("initializing network client");
        let client = reqwest::Client::builder()
            .use_native_tls()
            // envoy makes up totally bogus certs
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        let system_state = RwLock::new(SystemState::default());
        let inventory = RwLock::new(Inventory::default());
        let mut time_series = TimeSeriesData::default();
        let mut db = rusqlite::Connection::open(store_path)?;
        tracing::debug!(?store_path, "initializing time-series database");
        db.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS history(
                kind INTEGER NOT NULL,
                timestamp BIGINT NOT NULL,
                value BIGINT NOT NULL,
                PRIMARY KEY (kind, timestamp)
            );
            CREATE INDEX IF NOT EXISTS idx_history_on_timestamp ON history(timestamp);
            "#,
        )?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        time_series.load_from_db(&mut db)?;
        let time_series = RwLock::new(time_series);
        let db = Arc::new(Mutex::new(db));
        Ok(Self {
            client,
            system_state,
            inventory,
            time_series,
            db,
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
        *state_guard = new_state.clone();
        drop(state_guard);

        let db = self.db.clone();
        if let Err(err) = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut db = db.lock().unwrap();
            let tx = db.transaction()?;
            tx.execute(
                "INSERT INTO history(kind, timestamp, value) VALUES(?1, ?2, ?3)",
                (HistoryKind::Pv as u8, dt.timestamp(), new_state.pv_mw),
            )?;
            tx.execute(
                "INSERT INTO history(kind, timestamp, value) VALUES(?1, ?2, ?3)",
                (HistoryKind::Grid as u8, dt.timestamp(), new_state.grid_mw),
            )?;
            tx.execute(
                "INSERT INTO history(kind, timestamp, value) VALUES(?1, ?2, ?3)",
                (HistoryKind::Load as u8, dt.timestamp(), new_state.load_mw),
            )?;
            tx.execute(
                "INSERT INTO history(kind, timestamp, value) VALUES(?1, ?2, ?3)",
                (
                    HistoryKind::Storage as u8,
                    dt.timestamp(),
                    new_state.storage_mw,
                ),
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        {
            tracing::warn!(?err, "failed writing to database");
        }
    }

    pub async fn maintain(&self) {
        let mut time_series_guard = self.time_series.write().await;
        time_series_guard.pv_mw.maintain();
        time_series_guard.grid_mw.maintain();
        time_series_guard.load_mw.maintain();
        time_series_guard.storage_mw.maintain();
        drop(time_series_guard);
        let db = self.db.clone();
        const HISTORY_RETAIN: TimeDelta = TimeDelta::days(14);
        let threshold = Utc::now() - HISTORY_RETAIN;
        if let Err(err) = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut db = db.lock().unwrap();
            let tx = db.transaction()?;
            tx.execute(
                "DELETE FROM history WHERE timestamp < ?1",
                [threshold.timestamp()],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        {
            tracing::warn!(?err, "failed writing to database");
        }
    }
}
