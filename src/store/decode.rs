//! Helpers shared by the readers that decode the shared store's rows into the
//! board's types: the decode-fallback gauge and the timestamp parser.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

/// Process-wide count of decode soft-fails: defaulted enum values plus rows
/// dropped by a bulk read. See [`decode_fallback_count`].
static DECODE_FALLBACKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bump the decode-fallback counter and return the new total, so callers can
/// include `count=N` in their `tracing::warn!`. Call this from every soft-fail
/// branch — see the soft-fail-decoding section of `docs/conventions.md`.
///
/// Call it on its own line, **not** inline as a `tracing::warn!` field value:
/// the macro skips evaluating its field expressions when no subscriber has the
/// event enabled, so an inline bump would silently stop counting in every
/// process without a subscriber (most one-shot CLI subcommands, and the test
/// suite).
pub(crate) fn bump_decode_fallback() -> u64 {
    DECODE_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Number of decode soft-fails since process start: unknown enum values that
/// were defaulted, plus rows skipped by a bulk read because they could not be
/// decoded. Monotonic and never reset — compare deltas, not absolutes (the
/// test suite shares one process). See the decode-failure-policy section of
/// `docs/conventions.md`.
pub fn decode_fallback_count() -> u64 {
    DECODE_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Drop a row of `kind` that the shared store sent and the board cannot
/// decode. Counted as well as logged: [`decode_fallback_count`] is the one
/// number that says a board is quietly dropping rows.
pub(crate) fn drop_undecodable(kind: &str, error: &dyn std::fmt::Display) {
    let count = bump_decode_fallback();
    tracing::warn!(
        count,
        "dropping an undecodable {kind} from the shared store: {error}"
    );
}

/// Parse a stored timestamp: "YYYY-MM-DD HH:MM:SS", with optional fractional
/// seconds. The store's rows carry this text, and `crate::sync::decode` reads
/// it back through here, so there is one parser for the one format.
pub(crate) fn parse_datetime(s: &str) -> Result<DateTime<Utc>, String> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .map(|ndt| Utc.from_utc_datetime(&ndt))
        .map_err(|e| format!("invalid datetime {s:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_datetime_accepts_whole_and_fractional_seconds() {
        assert!(parse_datetime("2026-01-02 03:04:05").is_ok());
        assert!(parse_datetime("2026-01-02 03:04:05.123").is_ok());
    }

    #[test]
    fn parse_datetime_rejects_garbage_naming_the_input() {
        let err = parse_datetime("yesterday").unwrap_err();
        assert!(err.contains("yesterday"), "{err}");
    }

    #[test]
    fn bump_raises_the_count() {
        let before = decode_fallback_count();
        bump_decode_fallback();
        assert!(decode_fallback_count() > before);
    }
}
