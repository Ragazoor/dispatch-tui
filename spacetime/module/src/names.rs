//! Agent names: the picker and the format check.
//!
//! Spec: `agents.allium: AgentNameIsNotAnId`. Pure functions, so they are
//! testable without a `ReducerContext`.

/// The longest agent name.
pub const MAX_NAME_LEN: usize = 24;

/// Is `name` an acceptable agent name? Lowercase letters, digits and hyphens, not
/// empty, not too long, and not only digits (a string of digits reads as a
/// task id — `agents.allium: AgentNameIsNotAnId`).
pub fn validate_agent_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a name cannot be empty".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "name {name:?} is longer than {MAX_NAME_LEN} characters"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "name {name:?} may only hold lowercase letters, digits and hyphens"
        ));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "name {name:?} is only digits and would read as a task id"
        ));
    }
    Ok(())
}

/// The agent-name picker's walk
/// (`agents.allium: StartAgentOnFreshLaunch`): start at `seed` in `words`,
/// take the first word `is_taken` does not claim, and when every word is
/// taken append 2, 3, ... to the word at `seed`. `words` must not be empty.
pub fn pick_word(words: &[&str], seed: u64, is_taken: impl Fn(&str) -> bool) -> String {
    let n = words.len();
    let start = (seed % n as u64) as usize;
    for offset in 0..n {
        let word = words[(start + offset) % n];
        if !is_taken(word) {
            return word.to_string();
        }
    }
    let base = words[start];
    (2u64..)
        .map(|k| format!("{base}{k}"))
        .find(|candidate| !is_taken(candidate))
        .expect("an unbounded counter always finds an unused word")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: &[&str] = &["ada", "linus", "grace", "alan", "edsger", "barbara"];

    #[test]
    fn a_word_is_a_valid_name() {
        assert!(validate_agent_name("ada").is_ok());
        assert!(validate_agent_name("blue-fox").is_ok());
        assert!(validate_agent_name("ada2").is_ok());
    }

    #[test]
    fn digits_only_is_refused_because_it_reads_as_an_id() {
        assert!(validate_agent_name("65542").is_err());
        assert!(validate_agent_name("7").is_err());
    }

    #[test]
    fn empty_uppercase_spaces_and_overlong_are_refused() {
        assert!(validate_agent_name("").is_err());
        assert!(validate_agent_name("Ada").is_err());
        assert!(validate_agent_name("two words").is_err());
        assert!(validate_agent_name("a#b").is_err());
        assert!(validate_agent_name(&"a".repeat(MAX_NAME_LEN + 1)).is_err());
        assert!(validate_agent_name(&"a".repeat(MAX_NAME_LEN)).is_ok());
    }

    #[test]
    fn the_pick_starts_at_the_seed() {
        let n = WORDS.len() as u64;
        assert_eq!(pick_word(WORDS, 3, |_| false), WORDS[3]);
        assert_eq!(pick_word(WORDS, n + 3, |_| false), WORDS[3]);
    }

    #[test]
    fn a_taken_word_is_skipped() {
        let first = WORDS[3];
        assert_eq!(pick_word(WORDS, 3, |w| w == first), WORDS[4]);
    }

    #[test]
    fn the_walk_wraps_around_the_list() {
        let last = WORDS.len() as u64 - 1;
        let taken = WORDS[last as usize];
        assert_eq!(pick_word(WORDS, last, |w| w == taken), WORDS[0]);
    }

    #[test]
    fn with_every_word_taken_a_number_is_appended() {
        let base = WORDS[5];
        let all: std::collections::HashSet<String> =
            WORDS.iter().map(|w| w.to_string()).collect();
        assert_eq!(pick_word(WORDS, 5, |w| all.contains(w)), format!("{base}2"));
        let mut more = all.clone();
        more.insert(format!("{base}2"));
        assert_eq!(pick_word(WORDS, 5, |w| more.contains(w)), format!("{base}3"));
    }
}
