// SPDX-License-Identifier: GPL-3.0-or-later

//! Measures transfer speed over a sliding time window.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateMeter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl RateMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            samples: VecDeque::new(),
        }
    }

    /// Records `bytes` transferred at `now`.
    pub fn add(&mut self, now: Instant, bytes: u64) {
        self.samples.push_back((now, bytes));
        while let Some(&(at, _)) = self.samples.front() {
            if self.in_window(at, now) {
                break;
            }
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the window ending at `now`.
    pub fn rate(&self, now: Instant) -> u64 {
        let bytes: u64 = self
            .samples
            .iter()
            .filter(|&&(at, _)| self.in_window(at, now))
            .map(|&(_, n)| n)
            .sum();
        (bytes as u128 * 1000 / self.window.as_millis().max(1)) as u64
    }

    fn in_window(&self, at: Instant, now: Instant) -> bool {
        now.saturating_duration_since(at) < self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(2);

    #[test]
    fn empty_meter_reads_zero() {
        assert_eq!(RateMeter::new(WINDOW).rate(Instant::now()), 0);
    }

    #[test]
    fn rate_is_bytes_in_the_window_divided_by_the_window() {
        let t0 = Instant::now();
        let mut m = RateMeter::new(WINDOW);
        m.add(t0, 1_000_000);
        m.add(t0 + Duration::from_millis(500), 1_000_000);
        // 2 MB within a 2 s window: 1 MB/s.
        assert_eq!(m.rate(t0 + Duration::from_secs(1)), 1_000_000);
    }

    #[test]
    fn samples_older_than_the_window_stop_counting() {
        let t0 = Instant::now();
        let mut m = RateMeter::new(WINDOW);
        m.add(t0, 4_000_000);
        m.add(t0 + Duration::from_millis(2500), 1_000_000);
        // At t0 + 3 s, only the second sample is inside the window.
        assert_eq!(m.rate(t0 + Duration::from_secs(3)), 500_000);
        // Long after, nothing is.
        assert_eq!(m.rate(t0 + Duration::from_secs(10)), 0);
    }

    #[test]
    fn adding_prunes_old_samples_so_memory_stays_bounded() {
        let t0 = Instant::now();
        let mut m = RateMeter::new(WINDOW);
        for i in 0..10_000u64 {
            m.add(t0 + Duration::from_millis(i * 10), 1);
        }
        // 10 ms apart, so at most ~200 samples fit in a 2 s window.
        assert!(m.samples.len() <= 201, "{} samples kept", m.samples.len());
    }
}
