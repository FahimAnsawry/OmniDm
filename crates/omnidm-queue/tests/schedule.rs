// SPDX-License-Identifier: GPL-3.0-or-later

use jiff::Zoned;
use jiff::civil::{Time, Weekday, date, time};
use omnidm_queue::schedule::{Days, Schedule, Start};

/// New York, so the tests can cross real daylight-saving changes. In 2026 the clocks jump
/// from 02:00 to 03:00 on 8 March and fall back from 02:00 to 01:00 on 1 November.
fn at(text: &str) -> Zoned {
    format!("{text}[America/New_York]").parse().unwrap()
}

fn daily(hour: i8, minute: i8, days: Days) -> Schedule {
    Schedule {
        start: Some(Start::Daily {
            at: time(hour, minute, 0, 0),
            days,
        }),
        stop_at: None,
        wake: false,
    }
}

#[test]
fn daily_start_later_today() {
    let next = daily(2, 0, Days::EVERY_DAY).next_start(&at("2026-06-10T01:00"));
    assert_eq!(next, Some(at("2026-06-10T02:00")));
}

#[test]
fn a_start_at_exactly_now_is_tomorrow() {
    // Strictly after, so a queue that just started isn't started again.
    let next = daily(2, 0, Days::EVERY_DAY).next_start(&at("2026-06-10T02:00"));
    assert_eq!(next, Some(at("2026-06-11T02:00")));
}

#[test]
fn weekdays_skip_the_weekend() {
    // 12 June 2026 is a Friday.
    let next = daily(2, 0, Days::WEEKDAYS).next_start(&at("2026-06-12T03:00"));
    assert_eq!(next, Some(at("2026-06-15T02:00")));
    assert_eq!(next.unwrap().weekday(), Weekday::Monday);
}

#[test]
fn a_single_day_seen_after_its_time_waits_a_week() {
    // 15 June 2026 is a Monday.
    let mondays = Days::NONE.with(Weekday::Monday);
    let next = daily(2, 0, mondays).next_start(&at("2026-06-15T09:00"));
    assert_eq!(next, Some(at("2026-06-22T02:00")));
}

#[test]
fn no_days_never_starts() {
    assert_eq!(
        daily(2, 0, Days::NONE).next_start(&at("2026-06-10T01:00")),
        None
    );
}

#[test]
fn no_start_never_starts() {
    assert_eq!(
        Schedule::default().next_start(&at("2026-06-10T01:00")),
        None
    );
}

#[test]
fn once_fires_only_in_the_future() {
    let once = Schedule {
        start: Some(Start::Once(date(2026, 6, 10).at(23, 30, 0, 0))),
        stop_at: None,
        wake: false,
    };
    assert_eq!(
        once.next_start(&at("2026-06-10T12:00")),
        Some(at("2026-06-10T23:30"))
    );
    assert_eq!(once.next_start(&at("2026-06-10T23:30")), None);
    assert_eq!(once.next_start(&at("2026-06-11T00:00")), None);
}

#[test]
fn a_skipped_time_fires_right_after_the_jump() {
    // 02:30 doesn't exist on 8 March 2026; 03:30 daylight time is the same moment it
    // would have been.
    let next = daily(2, 30, Days::EVERY_DAY).next_start(&at("2026-03-08T00:00"));
    let next = next.unwrap();
    assert_eq!(
        next.to_string(),
        "2026-03-08T03:30:00-04:00[America/New_York]"
    );
}

#[test]
fn a_repeated_time_fires_once() {
    // 01:30 happens twice on 1 November 2026: first at -04:00, then at -05:00.
    let schedule = daily(1, 30, Days::EVERY_DAY);
    let first = schedule.next_start(&at("2026-11-01T00:00")).unwrap();
    assert_eq!(
        first.to_string(),
        "2026-11-01T01:30:00-04:00[America/New_York]"
    );
    // Asked again just after it fired, the second 01:30 is skipped.
    let after_first = first.checked_add(jiff::Span::new().minutes(1)).unwrap();
    let next = schedule.next_start(&after_first).unwrap();
    assert_eq!(
        next.to_string(),
        "2026-11-02T01:30:00-05:00[America/New_York]"
    );
}

#[test]
fn stop_is_the_next_time_on_any_day() {
    let schedule = Schedule {
        start: None,
        stop_at: Some(time(6, 0, 0, 0)),
        wake: false,
    };
    assert_eq!(
        schedule.next_stop(&at("2026-06-10T02:00")),
        Some(at("2026-06-10T06:00"))
    );
    assert_eq!(
        schedule.next_stop(&at("2026-06-10T07:00")),
        Some(at("2026-06-11T06:00"))
    );
    assert_eq!(Schedule::default().next_stop(&at("2026-06-10T07:00")), None);
}

#[test]
fn days_parse_names_and_groups() {
    assert_eq!("daily".parse::<Days>().unwrap(), Days::EVERY_DAY);
    assert_eq!("weekdays".parse::<Days>().unwrap(), Days::WEEKDAYS);
    assert_eq!("Sat, sun".parse::<Days>().unwrap(), Days::WEEKEND);
    let days: Days = "mon,wed".parse().unwrap();
    assert!(days.contains(Weekday::Monday));
    assert!(days.contains(Weekday::Wednesday));
    assert!(!days.contains(Weekday::Tuesday));
    assert_eq!(days.to_string(), "mon,wed");
    assert!("mon,funday".parse::<Days>().is_err());
}

#[test]
fn schedule_stores_as_readable_json() {
    let schedule = Schedule {
        start: Some(Start::Daily {
            at: time(2, 0, 0, 0),
            days: "mon,fri".parse().unwrap(),
        }),
        stop_at: Some(Time::constant(6, 30, 0, 0)),
        wake: false,
    };
    let json = serde_json::to_string(&schedule).unwrap();
    assert_eq!(
        json,
        r#"{"start":{"daily":{"at":"02:00:00","days":["mon","fri"]}},"stop_at":"06:30:00"}"#
    );
    assert_eq!(serde_json::from_str::<Schedule>(&json).unwrap(), schedule);

    let once = Schedule {
        start: Some(Start::Once(date(2026, 6, 10).at(23, 30, 0, 0))),
        stop_at: None,
        wake: false,
    };
    let json = serde_json::to_string(&once).unwrap();
    assert_eq!(json, r#"{"start":{"once":"2026-06-10T23:30:00"}}"#);
    assert_eq!(serde_json::from_str::<Schedule>(&json).unwrap(), once);
    assert_eq!(
        serde_json::from_str::<Schedule>("{}").unwrap(),
        Schedule::default()
    );
}
