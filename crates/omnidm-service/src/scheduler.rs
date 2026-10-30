// SPDX-License-Identifier: GPL-3.0-or-later

//! Starts and stops queues at their scheduled times, wakes the PC for them, and keeps it
//! awake while downloads run (plan §11.2).
//!
//! Each tick looks at the time that passed since the last one. A start or stop that came
//! due in it is acted on, even late: a PC that slept through 02:00 without a wake timer
//! starts the queue when it wakes. When both came due (it slept through the start *and*
//! the stop), the later one wins, so the queue doesn't start after its stop time. Times
//! from before OmniDM started are not caught up on.

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp, Zoned};
use omnidm_queue::power::Power;

use crate::manager::Manager;

/// The longest the scheduler goes without looking at the clock: it also notices a changed
/// clock, a waking PC and downloads starting or ending (for keeping the PC awake).
pub const CHECK_EVERY: Duration = Duration::from_secs(10);
/// How long before a scheduled start the PC is woken, so the network is back in time.
pub const WAKE_EARLY: SignedDuration = SignedDuration::from_secs(60);

pub struct Scheduler {
    manager: Manager,
    power: Arc<dyn Power>,
    /// The last tick; everything due up to it has been dealt with.
    last: Option<Timestamp>,
    /// The wake time last given to `power`.
    wake: Option<Timestamp>,
    /// What `power` was last told about keeping the PC awake.
    awake: Option<bool>,
}

impl Scheduler {
    pub fn new(manager: Manager, power: Arc<dyn Power>) -> Self {
        Self {
            manager,
            power,
            last: None,
            wake: None,
            awake: None,
        }
    }

    /// Starts and stops the queues whose times came since the last tick, sets the next
    /// wake-up and keeps the PC awake while anything downloads. Returns the next
    /// scheduled start or stop.
    pub fn tick(&mut self, now: &Zoned) -> Option<Timestamp> {
        let since = match self.last {
            Some(last) => last.to_zoned(now.time_zone().clone()),
            None => now.clone(),
        };
        let due = |at: Option<Zoned>| {
            at.map(|at| at.timestamp())
                .filter(|&at| at <= now.timestamp())
        };
        let mut next = None;
        let mut wake = None;
        for (queue, schedule) in self.manager.queue_schedules() {
            let start = due(schedule.next_start(&since));
            let stop = due(schedule.next_stop(&since));
            match (start, stop) {
                (Some(start), Some(stop)) if start > stop => self.manager.start_queue_id(queue),
                (_, Some(_)) => self.manager.stop_queue_id(queue),
                (Some(_), None) => self.manager.start_queue_id(queue),
                (None, None) => {}
            }
            let upcoming = schedule.next_start(now).map(|at| at.timestamp());
            next = earliest(next, upcoming);
            next = earliest(next, schedule.next_stop(now).map(|at| at.timestamp()));
            if schedule.wake {
                let early = upcoming.and_then(|at| at.checked_sub(WAKE_EARLY).ok());
                wake = earliest(wake, early);
            }
        }
        self.last = Some(now.timestamp());

        if wake != self.wake {
            match self.power.wake_at(wake) {
                Ok(()) => self.wake = wake,
                Err(e) => eprintln!("OmniDM: can't set the wake timer: {e}"),
            }
        }
        let busy = self.manager.has_running();
        if self.awake != Some(busy) {
            match self.power.keep_awake(busy) {
                Ok(()) => self.awake = Some(busy),
                Err(e) => eprintln!("OmniDM: can't keep the PC awake: {e}"),
            }
        }
        next
    }
}

fn earliest(a: Option<Timestamp>, b: Option<Timestamp>) -> Option<Timestamp> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Runs the scheduler for as long as the app runs.
pub async fn run(manager: Manager, power: Arc<dyn Power>) {
    let mut scheduler = Scheduler::new(manager.clone(), power);
    loop {
        let now = Zoned::now();
        let next = scheduler.tick(&now);
        // A little past the time, so the next tick finds it due.
        let wait = next
            .and_then(|at| Duration::try_from(at.duration_since(now.timestamp())).ok())
            .map_or(CHECK_EVERY, |until| {
                (until + Duration::from_millis(100)).min(CHECK_EVERY)
            });
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = manager.schedules_changed() => {}
        }
    }
}
