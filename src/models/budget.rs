//! Claude subscription rate-limit windows and the active model — the data
//! behind the top-row budget and model indicators (see docs/specs/
//! dispatch.allium: TokenBudgetIndicator, ActiveModelIndicator).
//!
//! Deliberately unrelated to `super::usage`: that module counts keybindings and
//! MCP tool calls. These are subscription budget windows.

use serde::{Deserialize, Serialize};

/// One rolling rate-limit window as reported by the statusLine hook payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BudgetWindow {
    pub used_percentage: f64,
    /// Unix epoch seconds at which this window resets.
    pub resets_at: i64,
}

impl BudgetWindow {
    /// Percentage constrained to 0..=100. The upstream field is documented as
    /// 0-100 but is not validated here. NaN values are treated as 0.0 to prevent
    /// nonsense colour or text — a missing/garbage reading should read as "no
    /// information", and 0 is the safe end for a used-percentage.
    pub fn clamped_percentage(&self) -> f64 {
        if self.used_percentage.is_nan() {
            0.0
        } else {
            self.used_percentage.clamp(0.0, 100.0)
        }
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            used_percentage: value.get("used_percentage")?.as_f64()?,
            resets_at: value.get("resets_at")?.as_i64()?,
        })
    }
}

/// Latest-wins snapshot of the account-global budget windows and active model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<BudgetWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<BudgetWindow>,
    /// The model's display_name, verbatim (see docs/specs/dispatch.allium:
    /// ActiveModelIndicator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Unix epoch seconds at which this snapshot was captured.
    pub captured_at: i64,
}

impl BudgetSnapshot {
    /// Extract the budget windows and active model from a statusLine hook
    /// payload.
    ///
    /// Returns `None` when the payload carries neither rate-limit windows nor
    /// a model — the normal steady state for a session that has not yet had
    /// an API response. A partially-specified window is dropped rather than
    /// defaulted, so an unknown percentage never renders as 0%.
    pub fn from_status_payload(payload: &serde_json::Value, captured_at: i64) -> Option<Self> {
        let limits = payload.get("rate_limits");
        let five_hour = limits
            .and_then(|limits| limits.get("five_hour"))
            .and_then(BudgetWindow::from_json);
        let seven_day = limits
            .and_then(|limits| limits.get("seven_day"))
            .and_then(BudgetWindow::from_json);
        let model = payload
            .get("model")
            .and_then(|m| m.get("display_name"))
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
        if five_hour.is_none() && seven_day.is_none() && model.is_none() {
            return None;
        }
        Some(Self {
            five_hour,
            seven_day,
            model,
            captured_at,
        })
    }

    /// Whether this snapshot is old enough to be dimmed in the top-row
    /// indicators (docs/specs/dispatch.allium: TokenBudgetIndicator's
    /// `DimmedWhenStale`, ActiveModelIndicator's guarantee of the same name).
    /// Shared so both indicators apply identical staleness semantics rather
    /// than two copies that could drift.
    pub fn is_stale(&self, now: i64, stale_after: std::time::Duration) -> bool {
        let age = now.saturating_sub(self.captured_at).max(0);
        age as u64 > stale_after.as_secs()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_both_windows() {
        let payload = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600_i64 },
                "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600_i64 }
            }
        });
        let snap = BudgetSnapshot::from_status_payload(&payload, 1738421000).unwrap();
        assert_eq!(snap.five_hour.unwrap().used_percentage, 23.5);
        assert_eq!(snap.seven_day.unwrap().resets_at, 1738857600);
        assert_eq!(snap.captured_at, 1738421000);
    }

    #[test]
    fn parses_five_hour_only() {
        let payload = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 10.0, "resets_at": 1_i64 }
            }
        });
        let snap = BudgetSnapshot::from_status_payload(&payload, 0).unwrap();
        assert!(snap.five_hour.is_some());
        assert!(snap.seven_day.is_none());
    }

    #[test]
    fn model_only_payload_produces_snapshot_with_no_windows() {
        // API-key and cloud-provider auth never emit rate_limits at all, but
        // the active model must still be captured (observability.allium:
        // StatusLineDecorator's SnapshotWrittenWhenEitherFieldPresent).
        let payload = json!({ "model": { "display_name": "Opus" } });
        let snap = BudgetSnapshot::from_status_payload(&payload, 0).unwrap();
        assert_eq!(snap.model.as_deref(), Some("Opus"));
        assert!(snap.five_hour.is_none());
        assert!(snap.seven_day.is_none());
    }

    #[test]
    fn neither_windows_nor_model_is_none() {
        let payload = json!({ "rate_limits": {} });
        assert!(BudgetSnapshot::from_status_payload(&payload, 0).is_none());
        assert!(BudgetSnapshot::from_status_payload(&json!({}), 0).is_none());
    }

    #[test]
    fn parses_model_alongside_windows() {
        let payload = json!({
            "model": { "display_name": "Sonnet 5" },
            "rate_limits": {
                "five_hour": { "used_percentage": 10.0, "resets_at": 1 }
            }
        });
        let snap = BudgetSnapshot::from_status_payload(&payload, 0).unwrap();
        assert_eq!(snap.model.as_deref(), Some("Sonnet 5"));
        assert!(snap.five_hour.is_some());
    }

    #[test]
    fn model_without_display_name_is_ignored() {
        let payload = json!({
            "model": {},
            "rate_limits": { "five_hour": { "used_percentage": 1.0, "resets_at": 1 } }
        });
        let snap = BudgetSnapshot::from_status_payload(&payload, 0).unwrap();
        assert!(snap.model.is_none());
    }

    #[test]
    fn window_missing_fields_is_skipped_not_defaulted() {
        // A window without used_percentage must not become 0% — that would
        // read as "plenty left" when we simply do not know.
        let payload = json!({
            "rate_limits": { "five_hour": { "resets_at": 5_i64 } }
        });
        assert!(BudgetSnapshot::from_status_payload(&payload, 0).is_none());
    }

    #[test]
    fn clamps_percentage_out_of_range() {
        let high = BudgetWindow {
            used_percentage: 137.0,
            resets_at: 0,
        };
        let low = BudgetWindow {
            used_percentage: -4.0,
            resets_at: 0,
        };
        assert_eq!(high.clamped_percentage(), 100.0);
        assert_eq!(low.clamped_percentage(), 0.0);
    }

    #[test]
    fn clamps_nan_to_zero() {
        let nan_window = BudgetWindow {
            used_percentage: f64::NAN,
            resets_at: 0,
        };
        assert_eq!(nan_window.clamped_percentage(), 0.0);
    }

    #[test]
    fn round_trips_through_json() {
        let snap = BudgetSnapshot {
            five_hour: Some(BudgetWindow {
                used_percentage: 1.5,
                resets_at: 2,
            }),
            seven_day: None,
            model: Some("Sonnet 5".to_string()),
            captured_at: 3,
        };
        let text = serde_json::to_string(&snap).unwrap();
        let back: BudgetSnapshot = serde_json::from_str(&text).unwrap();
        assert_eq!(snap, back);
    }
}
