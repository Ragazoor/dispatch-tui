use crate::models::FeedItem;

/// Deserialise a JSON byte slice as a `Vec<FeedItem>`.
pub(super) fn parse_feed_items(bytes: &[u8]) -> anyhow::Result<Vec<FeedItem>> {
    serde_json::from_slice(bytes).map_err(Into::into)
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
        let json =
            br#"[{"external_id":"1","title":"T","description":"","status":"backlog"}]"#;
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
}
