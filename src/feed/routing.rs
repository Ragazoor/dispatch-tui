use crate::models::{FeedRole, Signal};

/// Pure exclusion predicate (`ExcludeFromReviews` in `docs/specs/feeds.allium`):
/// true when a feed item must not reach the review subtree at all. Evaluated
/// BEFORE [`route`], so an excluded item is never routed, never inserted, and
/// never enters the reconcile's keep-set — an existing task for one is deleted
/// by the same stale pass that removes a merged PR.
///
/// Two independent rules, either enough on its own:
///
/// 1. **Own-authored** — `AuthorMe`. A PR the user wrote is not review work for
///    the user, unconditionally: nothing rescues it.
/// 2. **Settled approval** — `Approved` with neither `DirectRequest` nor
///    `TeamRequest`. The user has already approved and nobody has asked them to
///    look again. `Reviewed`, `Commented`, `AuthorBot` and `OrgReview` do NOT
///    rescue it — reviewing is how one comes to have approved.
///
/// The contract in the spec is the source of truth for WHY each rule is shaped
/// this way: what counts as an approval, why new commits after one do not bring
/// the card back, why the rescue set is the request signals rather than a
/// separately fetched pending-reviewer list, and why this lives in the runtime
/// rather than in the user-owned feed script. Read it before changing either
/// rule.
///
/// Only as good as the signals, all of which the feed script derives and all of
/// which soft-fail to absent rather than failing the feed. Every soft failure
/// errs toward VISIBLE — a missing `Approved` shows a settled PR, a missing
/// request signal also shows a PR — so none can hide outstanding review work.
pub fn excluded_from_reviews(signals: &[Signal]) -> bool {
    let has = |s: Signal| signals.contains(&s);
    if has(Signal::AuthorMe) {
        return true;
    }
    has(Signal::Approved) && !has(Signal::DirectRequest) && !has(Signal::TeamRequest)
}

/// Map a PR's signals to its target role sub-epic. Pure: no async, no DB, no
/// I/O. Precedence is documented in the PR-review-feed-routing design doc §3 —
/// engagement (a review or comment on a PR that is not my own) wins over the
/// bot-author rule, so a bot PR I have reviewed still routes to my reviews.
///
/// Total over the `Signal` set: every input (including the empty slice) maps to
/// exactly one of `MyReviews | TeamReviews | Bots`. Never returns
/// `None`/`ReviewsParent`/`Cve`.
///
/// `OrgReview` (an org-scoped review-related match, not limited to the
/// repos.conf repo list) is treated the same as `DirectRequest`: it still
/// loses to the bot rule, so an org-scoped bot PR routes to `Bots` unless it
/// is also reviewed/commented (engagement wins). `Approved` is ignored here
/// entirely — it is an exclusion input, not a routing hint.
///
/// The `!has(Signal::AuthorMe)` guard below is UNREACHABLE from the role-routed
/// path — [`excluded_from_reviews`] drops every `AuthorMe` item before routing.
/// It is retained rather than removed because `route` is total over the whole
/// signal set and must stay correct for an input this caller happens never to
/// hand it.
pub fn route(signals: &[Signal]) -> FeedRole {
    let has = |s: Signal| signals.contains(&s);
    let engaged = (has(Signal::Reviewed) || has(Signal::Commented)) && !has(Signal::AuthorMe);
    if engaged {
        FeedRole::MyReviews
    } else if has(Signal::AuthorBot) {
        FeedRole::Bots
    } else if has(Signal::DirectRequest) || has(Signal::OrgReview) {
        FeedRole::MyReviews
    } else if has(Signal::TeamRequest) {
        FeedRole::TeamReviews
    } else {
        FeedRole::MyReviews
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::models::FeedRole;
    use crate::models::Signal::*;

    #[test]
    fn direct_request_to_my() {
        assert_eq!(route(&[DirectRequest]), FeedRole::MyReviews);
    }
    #[test]
    fn team_request_to_team() {
        assert_eq!(route(&[TeamRequest]), FeedRole::TeamReviews);
    }
    #[test]
    fn reviewed_to_my() {
        assert_eq!(route(&[Reviewed]), FeedRole::MyReviews);
    }
    #[test]
    fn commented_to_my() {
        assert_eq!(route(&[Commented]), FeedRole::MyReviews);
    }
    #[test]
    fn bot_to_bots() {
        assert_eq!(route(&[AuthorBot]), FeedRole::Bots);
    }

    // engaged wins over bot (resolved decision #1)
    #[test]
    fn reviewed_bot_to_my() {
        assert_eq!(route(&[Reviewed, AuthorBot]), FeedRole::MyReviews);
    }
    // but my own commented PR is not "engagement" -> bot/author rules apply
    #[test]
    fn own_comment_on_bot_is_bots() {
        assert_eq!(route(&[Commented, AuthorMe, AuthorBot]), FeedRole::Bots);
    }
    // team-requested PR I reviewed -> My (engagement wins, no leak)
    #[test]
    fn reviewed_team_to_my() {
        assert_eq!(route(&[TeamRequest, Reviewed]), FeedRole::MyReviews);
    }
    // empty -> fallback My
    #[test]
    fn empty_to_my() {
        assert_eq!(route(&[]), FeedRole::MyReviews);
    }

    #[test]
    fn org_review_to_my() {
        assert_eq!(route(&[OrgReview]), FeedRole::MyReviews);
    }

    // an org-scoped match doesn't override the bot rule, matching the
    // existing direct_request-vs-bot precedent (author_bot is checked first).
    #[test]
    fn org_review_bot_to_bots() {
        assert_eq!(route(&[OrgReview, AuthorBot]), FeedRole::Bots);
    }

    // but engagement still wins over an org-scoped bot PR.
    #[test]
    fn org_review_bot_reviewed_to_my() {
        assert_eq!(
            route(&[OrgReview, AuthorBot, Reviewed]),
            FeedRole::MyReviews
        );
    }

    // --- excluded_from_reviews (ExcludeFromReviews) ---

    #[test]
    fn author_me_is_excluded() {
        assert!(excluded_from_reviews(&[AuthorMe]));
    }

    #[test]
    fn empty_is_not_excluded() {
        assert!(!excluded_from_reviews(&[]));
    }

    // NoOtherExclusion: a set with neither author_me nor approved is kept.
    #[test]
    fn no_other_signal_excludes() {
        for s in [
            DirectRequest,
            TeamRequest,
            Reviewed,
            Commented,
            AuthorBot,
            OrgReview,
        ] {
            assert!(
                !excluded_from_reviews(&[s]),
                "{s:?} must not exclude an item"
            );
        }
        assert!(!excluded_from_reviews(&[
            DirectRequest,
            TeamRequest,
            Reviewed,
            Commented,
            AuthorBot,
            OrgReview,
        ]));
    }

    // OwnAuthoredExcluded: no combination of other signals rescues an
    // author_me item. The exclusion is unconditional, NOT "drop from
    // my_reviews only" — and it outranks the approval rescue.
    #[test]
    fn author_me_excluded_alongside_every_other_signal() {
        for s in [
            DirectRequest,
            TeamRequest,
            Reviewed,
            Commented,
            AuthorBot,
            OrgReview,
            Approved,
        ] {
            assert!(
                excluded_from_reviews(&[AuthorMe, s]),
                "author_me + {s:?} must still be excluded"
            );
        }
        assert!(excluded_from_reviews(&[
            DirectRequest,
            TeamRequest,
            Reviewed,
            Commented,
            AuthorBot,
            AuthorMe,
            OrgReview,
            Approved,
        ]));
    }

    // --- SettledApprovalExcluded ---

    // The whole point of the feature: a PR I have already approved, with no
    // pending request for me, is not review work and must not appear.
    #[test]
    fn approved_alone_is_excluded() {
        assert!(excluded_from_reviews(&[Approved]));
    }

    // The signal that put it in My Reviews in the first place does not rescue
    // it — reviewing IS how you come to have approved it.
    #[test]
    fn approved_with_reviewed_is_excluded() {
        assert!(excluded_from_reviews(&[Approved, Reviewed]));
    }

    // Neither does commenting, an org-scoped match, or the PR being a bot's.
    #[test]
    fn approved_is_not_rescued_by_non_request_signals() {
        for s in [Reviewed, Commented, AuthorBot, OrgReview] {
            assert!(
                excluded_from_reviews(&[Approved, s]),
                "approved + {s:?} must still be excluded"
            );
        }
    }

    // Either request signal rescues it on its own: a review has been asked for
    // again, so there is work to do despite the approval. direct_request covers
    // a request addressed to me personally (repo- or org-scoped); team_request
    // covers one addressed to a team I belong to.
    #[test]
    fn each_request_signal_rescues_an_approved_pr() {
        for s in [DirectRequest, TeamRequest] {
            assert!(
                !excluded_from_reviews(&[Approved, s]),
                "approved + {s:?} must be rescued"
            );
        }
    }

    // A re-request after approval typically arrives with the reviewed signal
    // still attached, because `reviewed-by:@me` keeps matching.
    #[test]
    fn re_request_after_approval_is_kept() {
        assert!(!excluded_from_reviews(&[Approved, Reviewed, DirectRequest]));
        assert!(!excluded_from_reviews(&[Approved, Reviewed, TeamRequest]));
    }

    // An org-scoped personal re-request arrives as direct_request, not
    // org_review — which is what makes the rescue reach PRs outside the
    // repos.conf list. org_review alone must NOT rescue.
    #[test]
    fn org_review_alone_does_not_rescue_an_approved_pr() {
        assert!(excluded_from_reviews(&[Approved, OrgReview]));
        assert!(!excluded_from_reviews(&[
            Approved,
            OrgReview,
            DirectRequest
        ]));
    }

    // Rule 1 beats rule 2's rescue: an own-authored PR is dropped even when a
    // review is requested from me on it.
    #[test]
    fn author_me_beats_the_approval_rescue() {
        assert!(excluded_from_reviews(&[AuthorMe, Approved, DirectRequest]));
    }

    // route ignores the approval signal entirely (its Total/Pure invariants
    // still hold over the widened Signal set).
    #[test]
    fn route_ignores_the_approval_signal() {
        assert_eq!(route(&[Approved]), FeedRole::MyReviews);
        assert_eq!(route(&[Approved, AuthorBot]), FeedRole::Bots);
        assert_eq!(route(&[Approved, TeamRequest]), FeedRole::TeamReviews);
    }
}
