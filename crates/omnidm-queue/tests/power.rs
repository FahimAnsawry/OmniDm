// SPDX-License-Identifier: GPL-3.0-or-later

use jiff::{SignedDuration, Timestamp};
use omnidm_queue::power::filetime;

#[test]
fn filetime_counts_100ns_steps_since_1601() {
    assert_eq!(filetime(Timestamp::UNIX_EPOCH), 116_444_736_000_000_000);
    let later = Timestamp::UNIX_EPOCH + SignedDuration::from_nanos(1_500);
    assert_eq!(filetime(later), 116_444_736_000_000_015);
}

#[cfg(windows)]
#[test]
fn windows_accepts_the_wake_timer_and_the_power_request() {
    use omnidm_queue::power::{Power, SystemPower};

    let power = SystemPower::new().unwrap();
    // An hour ahead, then cancelled: nothing actually wakes.
    let soon = Timestamp::now() + SignedDuration::from_hours(1);
    power.wake_at(Some(soon)).unwrap();
    power.wake_at(None).unwrap();
    power.keep_awake(true).unwrap();
    power.keep_awake(false).unwrap();
}

/// Sleep and shutdown can't be tried for real here; this checks the permission they need.
#[cfg(windows)]
#[test]
fn this_account_may_sleep_and_shut_down_the_pc() {
    omnidm_queue::power::enable_shutdown_privilege().unwrap();
}
