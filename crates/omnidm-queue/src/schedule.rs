// SPDX-License-Identifier: GPL-3.0-or-later

//! When a queue starts and stops by itself.
//!
//! Times are wall-clock times in the PC's time zone: "02:00 on weekdays" stays 02:00 when
//! daylight saving time begins or ends. A time the clocks skip (02:30 on the night they
//! jump to 03:00) fires at the moment after the jump; a time they repeat fires once, the
//! first time round.

use std::fmt;
use std::str::FromStr;

use jiff::ToSpan;
use jiff::Zoned;
use jiff::civil::{Date, DateTime, Time, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// When the queue starts by itself; `None` means only by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Start>,
    /// The queue stops (its downloads pause) at this time of day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_at: Option<Time>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Start {
    /// Once, at this date and time.
    Once(DateTime),
    /// At `at` on each of `days`.
    Daily { at: Time, days: Days },
}

impl Schedule {
    /// The first start strictly after `after`, in `after`'s time zone.
    pub fn next_start(&self, after: &Zoned) -> Option<Zoned> {
        match self.start.as_ref()? {
            Start::Once(at) => {
                let at = after.time_zone().to_zoned(*at).ok()?;
                (at.timestamp() > after.timestamp()).then_some(at)
            }
            Start::Daily { at, days } => {
                next_time(after, *at, |date| days.contains(date.weekday()))
            }
        }
    }

    /// The first stop strictly after `after`, on any day.
    pub fn next_stop(&self, after: &Zoned) -> Option<Zoned> {
        next_time(after, self.stop_at?, |_| true)
    }
}

/// The first `time` strictly after `after` on a day `wanted` accepts, looking a week ahead.
fn next_time(after: &Zoned, time: Time, wanted: impl Fn(Date) -> bool) -> Option<Zoned> {
    // Eight days, so "every Monday" seen on a Monday after the time finds next Monday.
    for offset in 0..=7_i64 {
        let date = after.date().checked_add(offset.days()).ok()?;
        if !wanted(date) {
            continue;
        }
        let at = after.time_zone().to_zoned(date.to_datetime(time)).ok()?;
        if at.timestamp() > after.timestamp() {
            return Some(at);
        }
    }
    None
}

/// A set of weekdays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Days(u8);

const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Days {
    pub const NONE: Self = Self(0);
    pub const EVERY_DAY: Self = Self(0b111_1111);
    /// Monday to Friday.
    pub const WEEKDAYS: Self = Self(0b001_1111);
    pub const WEEKEND: Self = Self(0b110_0000);

    pub fn contains(self, day: Weekday) -> bool {
        self.0 & bit(day) != 0
    }

    pub fn with(self, day: Weekday) -> Self {
        Self(self.0 | bit(day))
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The days in the set, Monday first.
    pub fn iter(self) -> impl Iterator<Item = Weekday> {
        (0..7_i8)
            .filter_map(|i| Weekday::from_monday_zero_offset(i).ok())
            .filter(move |&day| self.contains(day))
    }
}

fn bit(day: Weekday) -> u8 {
    1 << day.to_monday_zero_offset()
}

fn day_name(day: Weekday) -> &'static str {
    DAY_NAMES[day.to_monday_zero_offset() as usize]
}

fn parse_day(name: &str) -> Option<Weekday> {
    let index = DAY_NAMES.iter().position(|&n| n == name)?;
    Weekday::from_monday_zero_offset(index as i8).ok()
}

/// Comma-separated: `mon,wed,fri`, or `daily`, `weekdays`, `weekend`.
impl FromStr for Days {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let mut days = Self::NONE;
        for part in text.split(',').map(|p| p.trim().to_ascii_lowercase()) {
            days = match part.as_str() {
                "daily" => Self::EVERY_DAY,
                "weekdays" => Self(days.0 | Self::WEEKDAYS.0),
                "weekend" => Self(days.0 | Self::WEEKEND.0),
                name => days.with(parse_day(name).ok_or_else(|| {
                    format!("unknown day {name:?} (use mon…sun, daily, weekdays or weekend)")
                })?),
            };
        }
        Ok(days)
    }
}

impl fmt::Display for Days {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.iter().map(day_name).collect();
        f.write_str(&names.join(","))
    }
}

/// Stored as a list of day names, `["mon", "tue"]`.
impl Serialize for Days {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(day_name))
    }
}

impl<'de> Deserialize<'de> for Days {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        names.iter().try_fold(Self::NONE, |days, name| {
            parse_day(name)
                .map(|day| days.with(day))
                .ok_or_else(|| serde::de::Error::custom(format!("unknown day {name:?}")))
        })
    }
}
