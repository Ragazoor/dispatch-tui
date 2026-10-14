//! Task aliases: the word list, the picker and the format check.
//!
//! Spec: `core.allium: TaskAliasIsASecondName`. Pure functions, so they are
//! testable without a `ReducerContext`; [`write_task`](crate::write_task)
//! is what makes them unavoidable.

/// The words an alias is drawn from. Short, lowercase, easy to tell apart at a
/// glance. Append-only in spirit: an alias already handed out never changes
/// when this list does, because a stored alias is not recomputed.
pub const ALIAS_WORDS: &[&str] = &[
    "amber", "anvil", "apple", "arrow", "aspen", "atlas", "badge", "basil", "beach", "birch",
    "blaze", "bloom", "bluff", "bolt", "brook", "cairn", "camel", "canoe", "cedar", "chalk",
    "charm", "cliff", "cloud", "clove", "cobra", "comet", "coral", "crane", "creek", "crest",
    "daisy", "delta", "dune", "eagle", "ember", "fable", "falcon", "fern", "finch", "fjord",
    "flame", "flint", "forge", "frost", "gale", "garnet", "glade", "grove", "harbor", "hazel",
    "heron", "hollow", "iris", "ivory", "jade", "jasper", "juniper", "kayak", "kelp", "lagoon",
    "lark", "laurel", "lemon", "lilac", "lotus", "lunar", "maple", "marsh", "meadow", "mesa",
    "mint", "moss", "nectar", "nova", "oak", "oasis", "olive", "onyx", "opal", "orbit", "otter",
    "pearl", "pebble", "pine", "plum", "polar", "quartz", "quill", "raven", "reef", "ridge",
    "river", "robin", "sable", "sage", "scar", "shore", "sierra", "slate", "spark", "spruce",
    "stone", "storm", "swan", "tango", "thistle", "tide", "topaz", "tulip", "umber", "vale",
    "velvet", "violet", "willow", "wren", "yarrow", "zephyr", "zinc",
];

/// The longest alias a person may choose.
pub const MAX_ALIAS_LEN: usize = 24;

/// Is `alias` an acceptable name? Lowercase letters, digits and hyphens, not
/// empty, not too long, and not only digits (a string of digits reads as a
/// task id — `core.allium: TaskAliasIsASecondName`).
pub fn validate_alias(alias: &str) -> Result<(), String> {
    if alias.is_empty() {
        return Err("an alias cannot be empty".into());
    }
    if alias.len() > MAX_ALIAS_LEN {
        return Err(format!(
            "alias {alias:?} is longer than {MAX_ALIAS_LEN} characters"
        ));
    }
    if !alias
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "alias {alias:?} may only hold lowercase letters, digits and hyphens"
        ));
    }
    if alias.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "alias {alias:?} is only digits and would read as a task id"
        ));
    }
    Ok(())
}

/// Pick a word for a new task. Starts at `seed` in the word list and walks
/// forward to the first word `is_taken` does not claim. When every word is
/// taken it appends a number to the word at `seed` (`scar2`, `scar3`, ...), so
/// the pick always terminates with an unused alias.
pub fn pick_alias(seed: u64, is_taken: impl Fn(&str) -> bool) -> String {
    let n = ALIAS_WORDS.len();
    let start = (seed % n as u64) as usize;
    for offset in 0..n {
        let word = ALIAS_WORDS[(start + offset) % n];
        if !is_taken(word) {
            return word.to_string();
        }
    }
    let base = ALIAS_WORDS[start];
    (2u64..)
        .map(|k| format!("{base}{k}"))
        .find(|candidate| !is_taken(candidate))
        .expect("an unbounded counter always finds an unused alias")
}

/// The alias a write to task `row_id` should store: `alias` itself once it is
/// well-formed and no other task holds it, or a picked word when it is empty.
/// `is_taken_by_other` answers for a candidate alias; it must ignore the task
/// being written (a task keeping its own alias is not a clash). Shared by the
/// module's `write_task` and the in-process twin so the two cannot drift.
pub fn resolve_alias(
    row_id: i64,
    alias: &str,
    seed: u64,
    is_taken_by_other: impl Fn(&str) -> bool,
) -> Result<String, String> {
    if alias.is_empty() {
        return Ok(pick_alias(seed, is_taken_by_other));
    }
    validate_alias(alias).map_err(|why| format!("task {row_id}: {why}"))?;
    if is_taken_by_other(alias) {
        return Err(format!(
            "task {row_id}: alias {alias:?} already names another task"
        ));
    }
    Ok(alias.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_word_is_a_valid_unique_alias() {
        let mut seen = std::collections::HashSet::new();
        for w in ALIAS_WORDS {
            assert!(validate_alias(w).is_ok(), "{w}");
            assert!(seen.insert(*w), "duplicate word {w}");
        }
    }

    #[test]
    fn a_word_is_a_valid_alias() {
        assert!(validate_alias("scar").is_ok());
        assert!(validate_alias("blue-fox").is_ok());
        assert!(validate_alias("scar2").is_ok());
    }

    #[test]
    fn digits_only_is_refused_because_it_reads_as_an_id() {
        assert!(validate_alias("65542").is_err());
        assert!(validate_alias("7").is_err());
    }

    #[test]
    fn empty_uppercase_spaces_and_overlong_are_refused() {
        assert!(validate_alias("").is_err());
        assert!(validate_alias("Scar").is_err());
        assert!(validate_alias("two words").is_err());
        assert!(validate_alias("a#b").is_err());
        assert!(validate_alias(&"a".repeat(MAX_ALIAS_LEN + 1)).is_err());
        assert!(validate_alias(&"a".repeat(MAX_ALIAS_LEN)).is_ok());
    }

    #[test]
    fn the_pick_starts_at_the_seed() {
        let n = ALIAS_WORDS.len() as u64;
        assert_eq!(pick_alias(3, |_| false), ALIAS_WORDS[3]);
        assert_eq!(pick_alias(n + 3, |_| false), ALIAS_WORDS[3]);
    }

    #[test]
    fn a_taken_word_is_skipped() {
        let first = ALIAS_WORDS[3];
        assert_eq!(pick_alias(3, |w| w == first), ALIAS_WORDS[4]);
    }

    #[test]
    fn the_walk_wraps_around_the_list() {
        let last = ALIAS_WORDS.len() as u64 - 1;
        let taken = ALIAS_WORDS[last as usize];
        assert_eq!(pick_alias(last, |w| w == taken), ALIAS_WORDS[0]);
    }

    #[test]
    fn with_every_word_taken_a_number_is_appended() {
        let base = ALIAS_WORDS[5];
        let all: std::collections::HashSet<String> =
            ALIAS_WORDS.iter().map(|w| w.to_string()).collect();
        assert_eq!(pick_alias(5, |w| all.contains(w)), format!("{base}2"));
        let mut more = all.clone();
        more.insert(format!("{base}2"));
        assert_eq!(pick_alias(5, |w| more.contains(w)), format!("{base}3"));
    }

    #[test]
    fn an_empty_alias_is_picked_and_a_free_one_is_kept() {
        assert_eq!(resolve_alias(1, "", 3, |_| false).unwrap(), ALIAS_WORDS[3]);
        assert_eq!(resolve_alias(1, "scar", 0, |_| false).unwrap(), "scar");
    }

    #[test]
    fn a_taken_or_malformed_alias_is_refused_naming_the_task() {
        let taken = resolve_alias(42, "scar", 0, |w| w == "scar").unwrap_err();
        assert!(
            taken.contains("task 42") && taken.contains("scar"),
            "{taken}"
        );
        let digits = resolve_alias(42, "12345", 0, |_| false).unwrap_err();
        assert!(digits.contains("task 42"), "{digits}");
    }

    #[test]
    fn a_pick_is_always_a_valid_alias() {
        for seed in [0u64, 1, 999, u64::MAX] {
            assert!(validate_alias(&pick_alias(seed, |_| false)).is_ok());
        }
    }
}
