//! People and display names (`docs/specs/workspaces.allium`: `ShownName`).

/// `workspaces.allium: config.identity_prefix_length`, in characters.
const IDENTITY_PREFIX_LENGTH: usize = 8;

/// What a person in no workspace is told when they try to create an epic or
/// a package (`workspaces.allium: NameWorkspaceAndVisibilityFromBoard`).
pub const NO_WORKSPACE: &str = "You belong to no workspace; ask a super-admin to add you";

/// A workspace as a person picks one: its store id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRef {
    pub id: i64,
    pub name: String,
}

/// How a person is named wherever a card or a tool result names them
/// (`CreatorIsShownByName`): their display name, or the identity's short
/// prefix when they have set none. The one place the board does this; the
/// module's `shown_name` is its twin and a test holds the two together.
pub fn shown_name(display_name: &str, identity: &str) -> String {
    if display_name.is_empty() {
        identity.chars().take(IDENTITY_PREFIX_LENGTH).collect()
    } else {
        display_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_name_is_shown() {
        assert_eq!(shown_name("Alice", "abcdef0123456789"), "Alice");
    }

    #[test]
    fn no_name_shows_the_eight_character_prefix() {
        assert_eq!(shown_name("", "abcdef0123456789"), "abcdef01");
    }

    #[test]
    fn an_identity_shorter_than_the_prefix_is_shown_whole() {
        assert_eq!(shown_name("", "abc"), "abc");
    }
}
