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
mod tests;
