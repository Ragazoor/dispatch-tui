//! Exponential backoff, in one place.
//!
//! Two subsystems retry a thing that keeps failing — the PR poller against
//! GitHub, and the board's connection to the shared store — and they want the
//! same curve with different constants. The curve is three lines, and it was
//! written twice before this module existed; the two copies had already drifted
//! on how they handled an overflowing shift.
//!
//! That drift is the reason this is shared rather than duplicated. The
//! dangerous mistake here is not getting the doubling wrong, it is letting a
//! very long outage overflow and wrap around to a SHORT wait — turning the
//! worst outage into the busiest retry loop, which is the opposite of what a
//! backoff is for. Saturating arithmetic is what rules that out, and one
//! implementation is what keeps it ruled out everywhere.

use std::time::Duration;

/// How long to wait before the retry that follows `attempts` consecutive
/// failures: `base`, doubled per attempt, never past `max`.
///
/// `attempts` counts from 1 for the first failure, so the first wait is `base`
/// itself. Zero is treated as one — a caller asking about a failure that has
/// not happened gets the shortest wait rather than a panic or a zero.
///
/// Saturating throughout. A long outage drives the exponent past what a
/// `Duration` can hold, and the answer there is the ceiling; nothing about a
/// large `attempts` may produce a small wait.
pub fn exponential(base: Duration, max: Duration, attempts: u32) -> Duration {
    base.saturating_mul(2u32.saturating_pow(attempts.saturating_sub(1)))
        .min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    #[test]
    fn the_first_wait_is_the_base_and_each_one_after_doubles() {
        assert_eq!(exponential(BASE, MAX, 1), BASE);
        assert_eq!(exponential(BASE, MAX, 2), BASE * 2);
        assert_eq!(exponential(BASE, MAX, 3), BASE * 4);
        assert_eq!(exponential(BASE, MAX, 4), BASE * 8);
    }

    /// A caller that has not failed yet asks about attempt zero. It gets the
    /// shortest wait, not a zero wait and not a panic.
    #[test]
    fn a_zeroth_attempt_waits_the_base() {
        assert_eq!(exponential(BASE, MAX, 0), BASE);
    }

    #[test]
    fn the_wait_never_exceeds_the_ceiling() {
        for attempts in 0..=64 {
            assert!(
                exponential(BASE, MAX, attempts) <= MAX,
                "attempt {attempts} waited {:?}",
                exponential(BASE, MAX, attempts)
            );
        }
    }

    /// The trap this module exists for. An exponent large enough to overflow
    /// must land on the ceiling, never wrap around to a short wait — a long
    /// outage must not become the busiest retry loop.
    #[test]
    fn an_overflowing_exponent_saturates_rather_than_wrapping() {
        assert_eq!(exponential(BASE, MAX, 32), MAX);
        assert_eq!(exponential(BASE, MAX, 33), MAX);
        assert_eq!(exponential(BASE, MAX, u32::MAX), MAX);
    }

    /// Monotonic: a longer outage never waits less than a shorter one.
    ///
    /// The property the overflow test checks at one point, checked across the
    /// whole range — it is what a reader actually assumes when they see
    /// "backoff".
    #[test]
    fn a_later_attempt_never_waits_less_than_an_earlier_one() {
        let mut previous = Duration::ZERO;
        for attempts in 0..=64 {
            let wait = exponential(BASE, MAX, attempts);
            assert!(wait >= previous, "attempt {attempts} went backwards");
            previous = wait;
        }
    }
}
