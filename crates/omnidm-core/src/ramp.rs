// SPDX-License-Identifier: GPL-3.0-or-later

//! Decides how many connections a download may add on each controller tick.
//!
//! Connections are added in rounds: at most one round per cooldown, growing the count
//! by doubling (or gently), never past the per-download maximum or the hard cap, and not
//! at all while a speed limit is already nearly used up.

use std::time::{Duration, Instant};

/// No download ever uses more connections than this.
pub const HARD_CAP: u32 = 32;

/// Share of a speed limit (in percent) at which no more connections are opened.
const SPEED_GATE_PERCENT: u64 = 85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RampMode {
    /// 1, 2, 4, 8, 16, 32
    Doubling,
    /// 1, 2, 3, 4, 6, 8, 12, 16, 24, 32: for servers that dislike bursts of connections.
    Gentle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampPolicy {
    pub max_connections: u32,
    pub mode: RampMode,
    pub cooldown: Duration,
}

impl Default for RampPolicy {
    fn default() -> Self {
        Self {
            max_connections: 8,
            mode: RampMode::Doubling,
            cooldown: Duration::from_secs(2),
        }
    }
}

/// What the controller needs to know about the download right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampInput {
    pub now: Instant,
    /// Connections currently open.
    pub active: u32,
    /// Current download speed in bytes per second.
    pub speed: u64,
    /// The speed limit that applies to this download, in bytes per second.
    pub speed_limit: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RampController {
    policy: RampPolicy,
    last_round: Option<Instant>,
}

impl RampController {
    pub fn new(policy: RampPolicy) -> Self {
        Self {
            policy,
            last_round: None,
        }
    }

    /// How many connections to add now. Doesn't change any state: call
    /// [`record_round`](Self::record_round) once connections were actually added.
    pub fn wanted(&self, input: RampInput) -> u32 {
        let max = self.policy.max_connections.clamp(1, HARD_CAP);
        if input.active >= max {
            return 0;
        }
        if input.active == 0 {
            return 1;
        }
        if let Some(last) = self.last_round
            && input.now.saturating_duration_since(last) < self.policy.cooldown
        {
            return 0;
        }
        if let Some(limit) = input.speed_limit
            && input.speed.saturating_mul(100) >= limit.saturating_mul(SPEED_GATE_PERCENT)
        {
            return 0;
        }
        let target = match self.policy.mode {
            RampMode::Doubling => input.active.saturating_mul(2),
            RampMode::Gentle => next_gentle_step(input.active),
        };
        target.min(max) - input.active
    }

    /// Starts the cooldown: called after a round actually added connections.
    pub fn record_round(&mut self, now: Instant) {
        self.last_round = Some(now);
    }
}

fn next_gentle_step(active: u32) -> u32 {
    const STEPS: [u32; 10] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32];
    STEPS
        .into_iter()
        .find(|&step| step > active)
        .unwrap_or(HARD_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(now: Instant, active: u32) -> RampInput {
        RampInput {
            now,
            active,
            speed: 0,
            speed_limit: None,
        }
    }

    fn controller(max_connections: u32, mode: RampMode) -> RampController {
        RampController::new(RampPolicy {
            max_connections,
            mode,
            cooldown: Duration::from_secs(2),
        })
    }

    #[test]
    fn first_round_doubles_one_connection_to_two() {
        let c = controller(8, RampMode::Doubling);
        assert_eq!(c.wanted(input(Instant::now(), 1)), 1);
    }

    #[test]
    fn a_download_without_connections_gets_one() {
        let c = controller(8, RampMode::Doubling);
        assert_eq!(c.wanted(input(Instant::now(), 0)), 1);
    }

    #[test]
    fn rounds_wait_for_the_cooldown() {
        let t0 = Instant::now();
        let mut c = controller(8, RampMode::Doubling);
        c.record_round(t0);
        assert_eq!(c.wanted(input(t0 + Duration::from_millis(1999), 2)), 0);
        assert_eq!(c.wanted(input(t0 + Duration::from_secs(2), 2)), 2);
    }

    #[test]
    fn asking_without_recording_does_not_start_the_cooldown() {
        let t0 = Instant::now();
        let c = controller(8, RampMode::Doubling);
        assert_eq!(c.wanted(input(t0, 1)), 1);
        assert_eq!(c.wanted(input(t0 + Duration::from_millis(100), 1)), 1);
    }

    #[test]
    fn doubling_stops_at_the_maximum() {
        let c = controller(8, RampMode::Doubling);
        let now = Instant::now();
        assert_eq!(c.wanted(input(now, 6)), 2);
        assert_eq!(c.wanted(input(now, 8)), 0);
        assert_eq!(c.wanted(input(now, 9)), 0);
    }

    #[test]
    fn gentle_mode_grows_by_the_documented_steps() {
        let c = controller(HARD_CAP, RampMode::Gentle);
        let now = Instant::now();
        // (active, expected additions): 1→2→3→4→6→8→12→16→24→32
        let steps = [
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 2),
            (6, 2),
            (8, 4),
            (12, 4),
            (16, 8),
            (24, 8),
        ];
        for (active, add) in steps {
            assert_eq!(c.wanted(input(now, active)), add, "from {active}");
        }
        assert_eq!(c.wanted(input(now, 32)), 0);
    }

    #[test]
    fn gentle_mode_from_an_odd_count_uses_the_next_step() {
        let c = controller(HARD_CAP, RampMode::Gentle);
        // 5 is not a step: the next step above it is 6.
        assert_eq!(c.wanted(input(Instant::now(), 5)), 1);
    }

    #[test]
    fn maximum_above_the_hard_cap_is_clamped() {
        let c = controller(50, RampMode::Doubling);
        let now = Instant::now();
        assert_eq!(c.wanted(input(now, 16)), 16);
        assert_eq!(c.wanted(input(now, 32)), 0);
    }

    #[test]
    fn no_connections_are_added_at_85_percent_of_the_speed_limit() {
        let c = controller(8, RampMode::Doubling);
        let now = Instant::now();
        let at = |speed| RampInput {
            now,
            active: 2,
            speed,
            speed_limit: Some(1000),
        };
        assert_eq!(c.wanted(at(850)), 0);
        assert_eq!(c.wanted(at(849)), 2);
    }
}
