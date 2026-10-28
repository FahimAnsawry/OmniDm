// SPDX-License-Identifier: GPL-3.0-or-later

//! The HTTP rules the engine relies on: reading `Content-Range`, checking that a server
//! answered the range that was asked for, `Retry-After`, retry delays and status classes.

use std::time::{Duration, SystemTime};

/// A parsed `Content-Range: bytes start-end/total` (`total` is `None` for `*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

/// The range a request asked for; `end` is inclusive, `None` means "to the end".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestedRange {
    pub start: u64,
    pub end: Option<u64>,
}

/// Why a response to a range request can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    /// `200`: the server sent the whole file (no range support, or `If-Range` failed
    /// because the file changed).
    Ignored,
    /// `416`: the range lies outside the file, which usually means the file shrank.
    Unsatisfiable,
    /// `206` without a readable `Content-Range`.
    MissingContentRange,
    /// `206` for a different range than requested.
    Mismatch(ContentRange),
    /// `206` reporting a different file size than before: the file changed.
    TotalChanged { expected: u64, found: u64 },
    /// Any other status.
    Status(u16),
}

/// Parses a `Content-Range` header of a `206` response.
pub fn parse_content_range(value: &str) -> Option<ContentRange> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = rest.trim().split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end: u64 = end.parse().ok()?;
    let total = match total {
        "*" => None,
        t => Some(t.parse::<u64>().ok()?),
    };
    if end < start || total.is_some_and(|t| end >= t) {
        return None;
    }
    Some(ContentRange { start, end, total })
}

/// Checks a response to a range request and returns the range it carries.
pub fn check_range_response(
    status: u16,
    content_range: Option<&str>,
    requested: RequestedRange,
    expected_total: Option<u64>,
) -> Result<ContentRange, RangeError> {
    match status {
        206 => {}
        200 => return Err(RangeError::Ignored),
        416 => return Err(RangeError::Unsatisfiable),
        other => return Err(RangeError::Status(other)),
    }
    let got = content_range
        .and_then(parse_content_range)
        .ok_or(RangeError::MissingContentRange)?;
    if let (Some(expected), Some(found)) = (expected_total, got.total)
        && expected != found
    {
        return Err(RangeError::TotalChanged { expected, found });
    }
    if got.start != requested.start || requested.end.is_some_and(|end| got.end > end) {
        return Err(RangeError::Mismatch(got));
    }
    Ok(got)
}

/// Parses `Retry-After` (delay in seconds, or an HTTP date relative to `now`).
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value.parse().ok().map(Duration::from_secs);
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

/// Exponential back-off with "equal jitter": `attempt` 1 waits about `base`, each further
/// attempt doubles it, up to `max`. `jitter` in `[0, 1)` picks a point in the upper half,
/// so retries from many connections don't line up.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration, jitter: f64) -> Duration {
    // 2^31 × base already exceeds any sensible maximum; capping keeps the shift in range.
    let doublings = attempt.saturating_sub(1).min(31);
    let delay = base.saturating_mul(1u32 << doublings).min(max);
    let half = delay / 2;
    half + half.mul_f64(jitter.clamp(0.0, 1.0))
}

/// What the engine should do with a response status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    Success,
    /// Temporary problem: retry with back-off.
    Retry,
    /// The server asks to slow down (429, 503): honour `Retry-After`, use fewer connections.
    SlowDown,
    /// Credentials needed (401, 407).
    AuthRequired,
    /// Retrying won't help (403, 404, 410, other 4xx).
    Fatal,
}

pub fn classify_status(status: u16) -> StatusClass {
    match status {
        200..=299 => StatusClass::Success,
        429 | 503 => StatusClass::SlowDown,
        401 | 407 => StatusClass::AuthRequired,
        408 | 425 | 500..=599 => StatusClass::Retry,
        _ => StatusClass::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: Option<u64>) -> RequestedRange {
        RequestedRange { start, end }
    }

    fn cr(start: u64, end: u64, total: Option<u64>) -> ContentRange {
        ContentRange { start, end, total }
    }

    #[test]
    fn content_range_with_a_known_total() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some(cr(0, 99, Some(1000)))
        );
        assert_eq!(
            parse_content_range("bytes 5000000000-5999999999/10000000000"),
            Some(cr(5_000_000_000, 5_999_999_999, Some(10_000_000_000)))
        );
    }

    #[test]
    fn content_range_with_an_unknown_total() {
        assert_eq!(parse_content_range("bytes 0-99/*"), Some(cr(0, 99, None)));
    }

    #[test]
    fn content_range_tolerates_case_and_spaces() {
        assert_eq!(
            parse_content_range("Bytes  10-19/20 "),
            Some(cr(10, 19, Some(20)))
        );
    }

    #[test]
    fn invalid_content_ranges_are_rejected() {
        for bad in [
            "",
            "bytes */1000",      // the 416 form carries no range
            "bytes 99-0/1000",   // end before start
            "bytes 0-1000/1000", // end beyond total
            "items 0-99/1000",
            "bytes 0-99",
            "bytes a-b/c",
            "bytes -5-99/1000",
        ] {
            assert_eq!(parse_content_range(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn matching_206_is_accepted() {
        assert_eq!(
            check_range_response(
                206,
                Some("bytes 100-199/1000"),
                range(100, Some(199)),
                Some(1000)
            ),
            Ok(cr(100, 199, Some(1000)))
        );
    }

    #[test]
    fn open_ended_request_accepts_any_end() {
        assert_eq!(
            check_range_response(206, Some("bytes 100-999/1000"), range(100, None), None),
            Ok(cr(100, 999, Some(1000)))
        );
    }

    #[test]
    fn server_may_send_less_than_a_bounded_request_asked_for() {
        // Servers can shorten a range (e.g. at the end of the file); the start must match.
        assert_eq!(
            check_range_response(
                206,
                Some("bytes 100-149/1000"),
                range(100, Some(199)),
                Some(1000)
            ),
            Ok(cr(100, 149, Some(1000)))
        );
    }

    #[test]
    fn a_206_for_another_start_or_a_longer_range_is_a_mismatch() {
        assert_eq!(
            check_range_response(
                206,
                Some("bytes 0-199/1000"),
                range(100, Some(199)),
                Some(1000)
            ),
            Err(RangeError::Mismatch(cr(0, 199, Some(1000))))
        );
        assert_eq!(
            check_range_response(
                206,
                Some("bytes 100-299/1000"),
                range(100, Some(199)),
                Some(1000)
            ),
            Err(RangeError::Mismatch(cr(100, 299, Some(1000))))
        );
    }

    #[test]
    fn a_206_with_a_new_total_means_the_file_changed() {
        assert_eq!(
            check_range_response(
                206,
                Some("bytes 100-199/2000"),
                range(100, Some(199)),
                Some(1000)
            ),
            Err(RangeError::TotalChanged {
                expected: 1000,
                found: 2000
            })
        );
    }

    #[test]
    fn a_206_without_content_range_is_rejected() {
        assert_eq!(
            check_range_response(206, None, range(0, None), None),
            Err(RangeError::MissingContentRange)
        );
        assert_eq!(
            check_range_response(206, Some("garbage"), range(0, None), None),
            Err(RangeError::MissingContentRange)
        );
    }

    #[test]
    fn a_200_means_the_range_was_ignored() {
        assert_eq!(
            check_range_response(200, None, range(100, Some(199)), Some(1000)),
            Err(RangeError::Ignored)
        );
    }

    #[test]
    fn a_416_is_unsatisfiable_and_other_statuses_pass_through() {
        assert_eq!(
            check_range_response(416, Some("bytes */500"), range(600, None), Some(1000)),
            Err(RangeError::Unsatisfiable)
        );
        assert_eq!(
            check_range_response(503, None, range(0, None), None),
            Err(RangeError::Status(503))
        );
    }

    #[test]
    fn retry_after_in_seconds() {
        let now = SystemTime::now();
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after(" 0 ", now), Some(Duration::ZERO));
    }

    #[test]
    fn retry_after_as_a_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // 1_700_000_090 = Tue, 14 Nov 2023 22:14:50 GMT, 90 s after `now`.
        assert_eq!(
            parse_retry_after("Tue, 14 Nov 2023 22:14:50 GMT", now),
            Some(Duration::from_secs(90))
        );
        // A date in the past means "retry now".
        assert_eq!(
            parse_retry_after("Tue, 14 Nov 2023 22:00:00 GMT", now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn invalid_retry_after_is_ignored() {
        let now = SystemTime::now();
        for bad in ["", "soon", "-5", "1.5"] {
            assert_eq!(parse_retry_after(bad, now), None, "{bad:?}");
        }
    }

    #[test]
    fn backoff_doubles_per_attempt_within_the_jitter_band() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(60);
        // Jitter 0 gives the bottom of the band (half the delay), jitter → 1 the top.
        let cases = [
            (1, 0.0, 500),
            (1, 0.999_999, 1000),
            (2, 0.0, 1000),
            (3, 0.0, 2000),
            (4, 0.5, 6000),
        ];
        for (attempt, jitter, want_ms) in cases {
            let got = backoff_delay(attempt, base, max, jitter).as_millis();
            assert!(
                got.abs_diff(want_ms) <= 1,
                "attempt {attempt}, jitter {jitter}: {got} ms, want {want_ms} ms"
            );
        }
    }

    #[test]
    fn backoff_never_exceeds_the_maximum() {
        let max = Duration::from_secs(60);
        for attempt in [7, 8, 20, 64, 1000, u32::MAX] {
            let d = backoff_delay(attempt, Duration::from_secs(1), max, 0.999_999);
            assert!(
                d <= max && d >= Duration::from_secs(30),
                "attempt {attempt}: {d:?}"
            );
        }
    }

    #[test]
    fn statuses_are_classified() {
        let cases = [
            (200, StatusClass::Success),
            (206, StatusClass::Success),
            (500, StatusClass::Retry),
            (502, StatusClass::Retry),
            (504, StatusClass::Retry),
            (408, StatusClass::Retry),
            (429, StatusClass::SlowDown),
            (503, StatusClass::SlowDown),
            (401, StatusClass::AuthRequired),
            (407, StatusClass::AuthRequired),
            (403, StatusClass::Fatal),
            (404, StatusClass::Fatal),
            (410, StatusClass::Fatal),
            (400, StatusClass::Fatal),
        ];
        for (status, want) in cases {
            assert_eq!(classify_status(status), want, "{status}");
        }
    }
}
