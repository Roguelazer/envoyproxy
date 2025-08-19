use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, TimeDelta, Timelike, Utc};
use serde::Serialize;

type Point = i64;

fn truncate_to_hour<H: chrono::TimeZone>(dt: &DateTime<H>) -> DateTime<H> {
    let time = dt.time();
    dt.with_time(chrono::NaiveTime::from_hms_opt(time.hour(), 0, 0).unwrap())
        .unwrap()
}

fn truncate_to_day<H: chrono::TimeZone>(dt: &DateTime<H>) -> DateTime<H> {
    dt.with_time(chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap())
        .unwrap()
}

fn truncate_to_week<H: chrono::TimeZone>(dt: &DateTime<H>) -> DateTime<H> {
    let weekday = dt.weekday();
    let start_of_week = dt.clone() - chrono::TimeDelta::days(weekday.number_from_monday() as i64);
    truncate_to_day(&start_of_week)
}

#[derive(Debug, Serialize, Clone)]
struct Statistics {
    average: Point,
    count: usize,
    max: Option<Point>,
    min: Option<Point>,
}

impl Statistics {
    fn from_points<'a>(values: impl IntoIterator<Item = &'a Point>) -> Self {
        let (sum, max, min, count) =
            values
                .into_iter()
                .fold((0, None, None, 0usize), |(sum, max, min, count), value| {
                    let new_max = match (max, value) {
                        (None, v) => Some(v),
                        (Some(v1), v2) if v1 < v2 => Some(v2),
                        (o, _) => o,
                    };
                    let new_min = match (min, value) {
                        (None, v) => Some(v),
                        (Some(v1), v2) if v1 > v2 => Some(v2),
                        (o, _) => o,
                    };
                    (sum + value, new_max, new_min, count + 1)
                });
        Self {
            average: sum / (count as i64),
            count,
            max: max.copied(),
            min: min.copied(),
        }
    }
}

#[derive(Debug, Default)]
pub struct TimeSeriesRow {
    raw_data: BTreeMap<DateTime<Utc>, Point>,
    hourly_data: BTreeMap<DateTime<Utc>, Statistics>,
    daily_data: BTreeMap<DateTime<Utc>, Statistics>,
    weekly_data: BTreeMap<DateTime<Utc>, Statistics>,
}

#[derive(Debug, Serialize)]
pub struct TimeSeriesSummary {
    hour: Option<Statistics>,
    day: Option<Statistics>,
    week: Option<Statistics>,
    last_24h: BTreeMap<chrono::DateTime<Utc>, Point>,
}

impl TimeSeriesRow {
    pub fn append<H: chrono::TimeZone>(&mut self, dt: DateTime<H>, datum: Point) {
        let utc = dt.with_timezone(&Utc);
        self.raw_data.insert(utc, datum);
        self.aggregate();
    }

    pub fn summary(&self) -> TimeSeriesSummary {
        let now = Utc::now();
        let hour = self.hourly_data.get(&truncate_to_hour(&now)).cloned();
        let day = self.daily_data.get(&truncate_to_day(&now)).cloned();
        let week = self.weekly_data.get(&truncate_to_week(&now)).cloned();

        let yesterday = now - TimeDelta::days(1);
        let last_24h = self
            .hourly_data
            .iter()
            .filter_map(|(d, s)| {
                if d >= &yesterday {
                    Some((*d, s.average))
                } else {
                    None
                }
            })
            .collect();

        TimeSeriesSummary {
            hour,
            day,
            week,
            last_24h,
        }
    }

    pub fn maintain(&mut self) {
        let threshold = Utc::now() - TimeDelta::days(14);
        self.raw_data.retain(|d, _| d >= &threshold);
    }

    fn aggregate_generic<F>(&mut self, f: F) -> Option<(DateTime<Utc>, Statistics)>
    where
        F: Fn(&DateTime<Utc>) -> DateTime<Utc>,
    {
        let schwartz = self
            .raw_data
            .iter()
            .map(|(d, p)| (f(d), d, p))
            .collect::<Vec<_>>();
        let current_truncated = schwartz.iter().map(|(h, _, _)| h).max().cloned()?;
        let current_values: BTreeMap<chrono::DateTime<Utc>, Point> = schwartz
            .iter()
            .filter_map(|(h, d, p)| {
                if *h == current_truncated {
                    Some((**d, **p))
                } else {
                    None
                }
            })
            .collect();
        Some((
            current_truncated,
            Statistics::from_points(current_values.values()),
        ))
    }

    fn aggregate(&mut self) {
        if let Some((dt, stat)) = self.aggregate_generic(truncate_to_hour) {
            self.hourly_data.insert(dt, stat);
        }
        if let Some((dt, stat)) = self.aggregate_generic(truncate_to_day) {
            self.daily_data.insert(dt, stat);
        }
        if let Some((dt, stat)) = self.aggregate_generic(truncate_to_week) {
            self.weekly_data.insert(dt, stat);
        }
    }
}
