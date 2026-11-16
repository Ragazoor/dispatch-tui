//! The `updated_*` and `completed_*` list filters, shared by `list_tasks`,
//! `list_epics` and `list_work_packages` (`mcp-task-tools.allium`:
//! `TimeFilterInput`).
//!
//! A bound is an ISO 8601 time with an explicit zone. `after` is inclusive,
//! `before` is exclusive, so two adjacent windows neither overlap nor leave a
//! gap. Output times are ISO 8601 UTC with a `Z` suffix.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

use super::types::{JsonRpcResponse, INVALID_PARAMS};

/// One field's window: both bounds optional.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct TimeWindow {
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
}

impl TimeWindow {
    /// Parse the two bounds of one field. `after_name` and `before_name` are
    /// the parameter names, so an error can name them.
    pub(super) fn parse(
        after_name: &str,
        after: Option<&str>,
        before_name: &str,
        before: Option<&str>,
    ) -> Result<Self, String> {
        let after_time = after.map(|s| parse_bound(after_name, s)).transpose()?;
        let before_time = before.map(|s| parse_bound(before_name, s)).transpose()?;
        if let (Some(a), Some(b)) = (after_time, before_time) {
            if a >= b {
                return Err(format!(
                    "{after_name} ({}) must be before {before_name} ({}): \
                     after is inclusive and before is exclusive",
                    iso(a),
                    iso(b)
                ));
            }
        }
        Ok(Self {
            after: after_time,
            before: before_time,
        })
    }

    /// True when neither bound is set: every row matches, even one with no
    /// time at all.
    pub(super) fn is_unbounded(&self) -> bool {
        self.after.is_none() && self.before.is_none()
    }

    /// Whether `time` falls in the window. A row with no time is excluded by
    /// any bound.
    pub(super) fn contains(&self, time: Option<DateTime<Utc>>) -> bool {
        if self.is_unbounded() {
            return true;
        }
        let Some(t) = time else { return false };
        self.after.is_none_or(|a| t >= a) && self.before.is_none_or(|b| t < b)
    }
}

fn parse_bound(name: &str, s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| {
            format!(
                "invalid {name}: {s:?}. Use an ISO 8601 time with a zone, \
                 like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00"
            )
        })
}

/// Parse one field's window into an `invalid params` response on error.
pub(super) fn parse_window(
    id: &Option<Value>,
    after_name: &str,
    after: Option<&str>,
    before_name: &str,
    before: Option<&str>,
) -> Result<TimeWindow, JsonRpcResponse> {
    TimeWindow::parse(after_name, after, before_name, before)
        .map_err(|msg| JsonRpcResponse::err(id.clone(), INVALID_PARAMS, msg))
}

/// Parse the `updated_*` and `completed_*` pairs of a list call. Each pair is
/// `(after, before)`.
pub(super) fn parse_windows(
    id: &Option<Value>,
    updated: (Option<&str>, Option<&str>),
    completed: (Option<&str>, Option<&str>),
) -> Result<(TimeWindow, TimeWindow), JsonRpcResponse> {
    let updated = parse_window(id, "updated_after", updated.0, "updated_before", updated.1)?;
    let completed = parse_window(
        id,
        "completed_after",
        completed.0,
        "completed_before",
        completed.1,
    )?;
    Ok((updated, completed))
}

/// ISO 8601 UTC with a `Z` suffix and whole seconds.
pub(super) fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// [`iso`], or `-` for a row with no time.
pub(super) fn iso_or_dash(t: Option<DateTime<Utc>>) -> String {
    t.map_or_else(|| "-".to_string(), iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn after_is_inclusive_and_before_is_exclusive() {
        let w = TimeWindow::parse(
            "a",
            Some("2026-11-09T00:00:00Z"),
            "b",
            Some("2026-11-16T00:00:00Z"),
        )
        .unwrap();
        assert!(w.contains(Some(at("2026-11-09T00:00:00Z"))));
        assert!(w.contains(Some(at("2026-11-15T23:59:59Z"))));
        assert!(!w.contains(Some(at("2026-11-16T00:00:00Z"))));
        assert!(!w.contains(Some(at("2026-11-08T23:59:59Z"))));
    }

    #[test]
    fn a_row_with_no_time_is_excluded_by_any_bound_only() {
        let bounded = TimeWindow::parse("a", Some("2026-11-09T00:00:00Z"), "b", None).unwrap();
        assert!(!bounded.contains(None));
        assert!(TimeWindow::default().contains(None));
    }

    #[test]
    fn a_numeric_offset_is_converted_to_utc() {
        let w = TimeWindow::parse("a", Some("2026-11-09T02:00:00+02:00"), "b", None).unwrap();
        assert!(w.contains(Some(at("2026-11-09T00:00:00Z"))));
        assert!(!w.contains(Some(at("2026-11-08T23:59:59Z"))));
    }

    #[test]
    fn a_time_without_a_zone_or_a_bare_date_is_invalid() {
        for bad in ["2026-11-09", "2026-11-09T00:00:00", "", "yesterday"] {
            let err = TimeWindow::parse("completed_after", Some(bad), "completed_before", None)
                .unwrap_err();
            assert!(err.contains("completed_after"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn a_range_where_after_is_not_before_before_names_both_parameters() {
        let t = "2026-11-09T00:00:00Z";
        for (a, b) in [("2026-11-16T00:00:00Z", t), (t, t)] {
            let err =
                TimeWindow::parse("updated_after", Some(a), "updated_before", Some(b)).unwrap_err();
            assert!(
                err.contains("updated_after") && err.contains("updated_before"),
                "{err}"
            );
        }
    }

    #[test]
    fn output_is_iso_utc_with_z_and_dash_for_none() {
        assert_eq!(iso(at("2026-11-11T08:26:37Z")), "2026-11-11T08:26:37Z");
        assert_eq!(iso_or_dash(None), "-");
    }
}
