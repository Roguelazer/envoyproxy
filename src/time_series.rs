use std::collections::BTreeMap;

use itertools::Itertools;
use jiff::{RoundMode, Span, Timestamp, Unit, ZonedDateTime, ZonedDateTimeRound};
use serde::Serialize;

type Point = i64;

fn truncate_to_hour(dt: &ZonedDateTime) -> ZonedDateTime {
    dt.round(
        ZonedDateTimeRound::new()
            .mode(RoundMode::Trunc)
            .smallest(Unit::Hour),
    )
    .unwrap()
}

fn truncate_to_day(dt: &ZonedDateTime) -> ZonedDateTime {
    dt.round(
        ZonedDateTimeRound::new()
            .mode(RoundMode::Trunc)
            .smallest(Unit::Day),
    )
    .unwrap()
}

fn truncate_to_week(dt: &ZonedDateTime) -> ZonedDateTime {
    let weekday = dt.weekday();
    let start_of_week = dt.clone() - Span::new().days(weekday.to_monday_zero_offset());
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
    raw_data: BTreeMap<ZonedDateTime, Point>,
    hourly_data: BTreeMap<ZonedDateTime, Statistics>,
    daily_data: BTreeMap<ZonedDateTime, Statistics>,
    weekly_data: BTreeMap<ZonedDateTime, Statistics>,
}

#[derive(Debug, Serialize)]
pub struct TimeSeriesSummary {
    hour: Option<Statistics>,
    day: Option<Statistics>,
    week: Option<Statistics>,
    last_24h: BTreeMap<Timestamp, Point>,
}

impl TimeSeriesRow {
    pub fn append(&mut self, dt: Timestamp, datum: Point) {
        let dt = dt.to_zoned_datetime(jiff::tz::TimeZone::UTC);
        self.append_raw(dt, datum);
        self.aggregate();
    }

    pub fn append_raw(&mut self, dt: ZonedDateTime, datum: Point) {
        self.raw_data.insert(dt, datum);
    }

    pub fn summary(&self) -> TimeSeriesSummary {
        let now = ZonedDateTime::now();
        let hour = self.hourly_data.get(&truncate_to_hour(&now)).cloned();
        let day = self.daily_data.get(&truncate_to_day(&now)).cloned();
        let week = self.weekly_data.get(&truncate_to_week(&now)).cloned();

        let yesterday = now.yesterday().unwrap();
        let last_24h = self
            .hourly_data
            .iter()
            .filter_map(|(d, s)| {
                if d >= yesterday {
                    Some((d.timestamp(), s.average))
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
        let threshold = ZonedDateTime::now() - Span::new().days(14);
        self.raw_data.retain(|d, _| d >= threshold);
    }

    fn aggregate_historical_generic<'a, D, F>(
        data: D,
        f: F,
    ) -> impl Iterator<Item = (ZonedDateTime, Statistics)> + use<'a, D, F>
    where
        F: Fn(&ZonedDateTime) -> ZonedDateTime,
        D: Iterator<Item = (&'a ZonedDateTime, &'a i64)>,
    {
        data.into_group_map_by(|r| f(r.0))
            .into_iter()
            .map(|(k, v)| (k, Statistics::from_points(v.into_iter().map(|v| v.1))))
    }

    fn aggregate_generic<F>(&self, f: F) -> Option<(ZonedDateTime, Statistics)>
    where
        F: Fn(&ZonedDateTime) -> ZonedDateTime,
    {
        let schwartz = self
            .raw_data
            .iter()
            .map(|(d, p)| (f(d), d, p))
            .collect::<Vec<_>>();
        let current_truncated = schwartz.iter().map(|(h, _, _)| h).max().cloned()?;
        let current_values: BTreeMap<ZonedDateTime, Point> = schwartz
            .iter()
            .filter_map(|(h, d, p)| {
                if *h == current_truncated {
                    Some(((*d).clone(), **p))
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

    pub fn aggregate_historical(&mut self) {
        self.hourly_data.extend(Self::aggregate_historical_generic(
            self.raw_data.iter(),
            truncate_to_hour,
        ));
        self.daily_data.extend(Self::aggregate_historical_generic(
            self.raw_data.iter(),
            truncate_to_day,
        ));
        self.weekly_data.extend(Self::aggregate_historical_generic(
            self.raw_data.iter(),
            truncate_to_week,
        ));
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

#[cfg(test)]
mod tests {
    use jiff::ZonedDateTime;

    #[test]
    fn test_truncate_to_hour() {
        let base: ZonedDateTime = "2026-11-19T22:05:00.123[UTC]".parse().unwrap();
        let expected: ZonedDateTime = "2026-11-19T22:00:00[UTC]".parse().unwrap();
        assert_eq!(super::truncate_to_hour(&base), expected);
    }

    #[test]
    fn test_truncate_to_day() {
        let base: ZonedDateTime = "2026-11-19T22:05:00.123[UTC]".parse().unwrap();
        let expected: ZonedDateTime = "2026-11-19T00:00:00[UTC]".parse().unwrap();
        assert_eq!(super::truncate_to_day(&base), expected);
    }

    #[test]
    fn test_truncate_to_week() {
        let base: ZonedDateTime = "2026-11-19T22:05:00.123[UTC]".parse().unwrap();
        let expected: ZonedDateTime = "2026-11-16T00:00:00[UTC]".parse().unwrap();
        assert_eq!(super::truncate_to_week(&base), expected);

        let base: ZonedDateTime = "2026-11-16T00:00:01.000[UTC]".parse().unwrap();
        let expected: ZonedDateTime = "2026-11-16T00:00:00[UTC]".parse().unwrap();
        assert_eq!(super::truncate_to_week(&base), expected);
    }
}
