use crate::models::FeedItem;

/// Deserialise a JSON byte slice as a `Vec<FeedItem>`.
///
/// The SINGLE feed-stdout decode, called by all three feed entry points:
/// [`crate::feed::FeedCycle::run`] (which both the auto-poll path and the
/// manual "r" refresh share) and `cmd_verify_feed` (the `verify-feed` CLI).
/// Three separate
/// `serde_json` call sites were three places a change to the feed wire format
/// could land in one path and not the others — the shape that produced the
/// `reviews_parent` parent-flat bug in the sync layer. See `feeds.allium`'s
/// `FeedItemParse` block.
///
/// Per-FIELD rules are not here: strict `tag`/`url_type` rejection and lenient
/// `signals` dropping are properties of [`FeedItem`]'s `Deserialize` impl, so
/// they apply to every caller identically and cannot diverge per path.
///
/// The one CROSS-FIELD rule is, because serde's field attributes cannot see a
/// sibling field and a shadow struct would duplicate every field. It runs via
/// [`FeedItem::validate`], and being at the single decode point it is just as
/// uniform: an offending item rejects the WHOLE emission, exactly as an unknown
/// `tag` does, because both are producer bugs a silent per-item drop would
/// hide. See `AReviewTaggedFeedItemNamesItsPr` in `docs/specs/feeds.allium`.
///
/// Presentation of the error stays with each caller.
pub fn parse_feed_items(bytes: &[u8]) -> anyhow::Result<Vec<FeedItem>> {
    let items: Vec<FeedItem> = serde_json::from_slice(bytes)?;
    for item in &items {
        item.validate().map_err(|e| anyhow::anyhow!(e))?;
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn valid_item_parsed() {
        let json =
            br#"[{"external_id":"1","title":"T","description":"D","status":"backlog","tag":"bug"}]"#;
        let items = parse_feed_items(json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "T");
        assert_eq!(items[0].external_id, "1");
    }

    #[test]
    fn empty_array_parsed() {
        let items = parse_feed_items(b"[]").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn missing_required_tag_fails() {
        let json = br#"[{"external_id":"1","title":"T","description":"","status":"backlog"}]"#;
        assert!(
            parse_feed_items(json).is_err(),
            "missing tag must fail deserialization"
        );
    }

    #[test]
    fn malformed_json_fails() {
        assert!(
            parse_feed_items(b"not-json").is_err(),
            "malformed JSON must fail"
        );
    }

    #[test]
    fn explicit_url_type_parsed_verbatim() {
        let json = br#"[{
            "external_id": "dependabot:org/repo#7",
            "title": "CVE-2026-1234",
            "description": "",
            "url": "https://github.com/org/repo/security/dependabot/7",
            "url_type": "security_alert",
            "status": "backlog",
            "tag": "fix"
        }]"#;
        let items = parse_feed_items(json).unwrap();
        assert_eq!(
            items[0].url_type,
            Some(crate::models::UrlType::SecurityAlert)
        );
    }

    #[test]
    fn omitted_url_type_defaults_to_none() {
        let json =
            br#"[{"external_id":"1","title":"T","description":"D","status":"backlog","tag":"bug"}]"#;
        let items = parse_feed_items(json).unwrap();
        assert_eq!(items[0].url_type, None, "wire compatibility: absent field");
    }

    #[test]
    fn unknown_url_type_fails() {
        let json = br#"[{"external_id":"1","title":"T","description":"","url_type":"bogus","status":"backlog","tag":"bug"}]"#;
        assert!(
            parse_feed_items(json).is_err(),
            "unknown url_type must fail deserialization, consistent with tag"
        );
    }

    #[test]
    fn unrecognised_signal_dropped_not_fatal() {
        // The single-item invariant lives with the type, in
        // src/models/tasks.rs::feed_item_signals_default_empty_and_unknown_skipped;
        // this asserts it survives the array path every feed caller uses.
        let json = br#"[{
            "external_id": "1",
            "title": "T",
            "description": "",
            "url": "https://github.com/o/r/pull/1",
            "status": "backlog",
            "tag": "pr-review",
            "signals": ["reviewed", "bogus"]
        }]"#;
        let items = parse_feed_items(json).unwrap();
        assert_eq!(items.len(), 1, "an unknown signal must not fail the item");
        assert_eq!(
            items[0].signals,
            vec![crate::models::Signal::Reviewed],
            "the unrecognised signal is dropped, the recognised one kept"
        );
    }

    #[test]
    fn author_label_at_prefix_preserved() {
        let json = br##"[{
            "external_id": "review:org/repo#7",
            "title": "#7 My PR",
            "description": "",
            "url": "https://github.com/org/repo/pull/7",
            "status": "backlog",
            "tag": "pr-review",
            "labels": ["@johndoe", "repo"]
        }]"##;
        let items = parse_feed_items(json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].labels,
            vec!["@johndoe".to_string(), "repo".to_string()]
        );
    }

    // -- AReviewTaggedFeedItemNamesItsPr (task #4728) --
    //
    // A review-tagged item must name a pull request. Rejection is
    // whole-emission, like an unknown `tag`: both are producer bugs, and a
    // silent drop hides a missing card. See feeds.allium's FeedItemParse.

    /// Every review tag, spelled as a feed script spells it. Derived from
    /// `ALL` filtered by `is_review` rather than listed, so a tag added to
    /// either — a new variant, or an existing one promoted into the review
    /// set — flows through this rule's coverage instead of quietly falling
    /// outside it. `task_tag_all_has_every_variant` and
    /// `task_tag_is_review_only_for_pr_review_and_dependabot` pin both inputs.
    fn review_tag_wire_names() -> Vec<&'static str> {
        use crate::models::TaskTag;
        TaskTag::ALL
            .iter()
            .copied()
            .filter(TaskTag::is_review)
            .map(TaskTag::as_str)
            .collect()
    }

    #[test]
    fn every_review_tag_requires_a_url() {
        for tag in review_tag_wire_names() {
            let json = format!(
                r#"[{{"external_id":"1","title":"T","description":"","status":"backlog","tag":"{tag}"}}]"#
            );
            assert!(
                parse_feed_items(json.as_bytes()).is_err(),
                "a {tag} item with no url must reject the emission"
            );
        }
    }

    #[test]
    fn a_review_tagged_item_with_a_pr_url_parses() {
        let json = br#"[{"external_id":"1","title":"T","description":"","url":"https://github.com/o/r/pull/42","status":"backlog","tag":"dependabot"}]"#;
        let items = parse_feed_items(json).expect("a pr url satisfies the rule");
        assert_eq!(items.len(), 1);
    }

    /// An issue url is not a PR. The rule is about naming a pull request, not
    /// about carrying any url at all.
    #[test]
    fn a_review_tagged_item_with_a_non_pr_url_rejects_the_emission() {
        let json = br#"[{"external_id":"1","title":"T","description":"","url":"https://github.com/o/r/issues/42","status":"backlog","tag":"pr-review"}]"#;
        assert!(
            parse_feed_items(json).is_err(),
            "an issue url must not satisfy a review tag"
        );
    }

    /// The rule reads the same resolved type ingest writes, so an explicitly
    /// declared `url_type` wins over inference here too.
    #[test]
    fn an_explicit_pr_url_type_satisfies_the_rule_where_inference_would_not() {
        let json = br#"[{"external_id":"1","title":"T","description":"","url":"https://example.com/review/42","url_type":"pr","status":"backlog","tag":"dependabot"}]"#;
        let items = parse_feed_items(json).expect("an explicit pr url_type satisfies the rule");
        assert_eq!(items.len(), 1);
    }

    /// The mirror of the above: a pr-shaped url explicitly declared something
    /// else does NOT satisfy the rule.
    #[test]
    fn an_explicit_non_pr_url_type_overrides_a_pr_shaped_url() {
        let json = br#"[{"external_id":"1","title":"T","description":"","url":"https://github.com/o/r/pull/42","url_type":"other","status":"backlog","tag":"dependabot"}]"#;
        assert!(
            parse_feed_items(json).is_err(),
            "an explicit url_type must be taken at its word, as ingest takes it"
        );
    }

    /// Nothing else is constrained. A CVE item legitimately carries a
    /// security_alert url, and a fix item may carry none at all.
    #[test]
    fn a_non_review_tag_is_unconstrained() {
        for item in [
            r#"{"external_id":"1","title":"T","description":"","status":"backlog","tag":"fix"}"#,
            r#"{"external_id":"2","title":"T","description":"","url":"https://github.com/o/r/security/dependabot/7","url_type":"security_alert","status":"backlog","tag":"fix"}"#,
            r#"{"external_id":"3","title":"T","description":"","url":"https://example.com/x","status":"backlog","tag":"chore"}"#,
        ] {
            let json = format!("[{item}]");
            assert!(
                parse_feed_items(json.as_bytes()).is_ok(),
                "a non-review tag must not be constrained: {item}"
            );
        }
    }

    /// Whole-emission, not per-item. One non-conforming item costs the cycle
    /// rather than costing one card — the same trade the strict `tag` rule
    /// makes, and the reason fetch-reviews.sh's single array is the blast
    /// radius the spec accepts.
    #[test]
    fn one_non_conforming_item_rejects_the_whole_array() {
        let json = br#"[
            {"external_id":"1","title":"good","description":"","url":"https://github.com/o/r/pull/1","status":"backlog","tag":"dependabot"},
            {"external_id":"2","title":"bad","description":"","status":"backlog","tag":"dependabot"}
        ]"#;
        assert!(
            parse_feed_items(json).is_err(),
            "a valid sibling must not rescue a non-conforming item"
        );
    }

    /// The error has to name the offender. A rejection that says only "invalid
    /// feed item" leaves the script author to bisect their own emission.
    #[test]
    fn the_rejection_names_the_item_and_the_tag() {
        let json = br#"[{"external_id":"dep:o/r#42","title":"T","description":"","status":"backlog","tag":"dependabot"}]"#;
        let err = parse_feed_items(json).expect_err("must reject").to_string();
        assert!(
            err.contains("dep:o/r#42"),
            "the error must name the offending external_id, got: {err}"
        );
        assert!(
            err.contains("dependabot"),
            "the error must name the tag that triggered the rule, got: {err}"
        );
    }
}
