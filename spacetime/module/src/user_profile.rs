//! People and display names (`workspaces.allium`, "People and display names").

use super::*;

/// `workspaces.allium: config.display_name_max_length`, in characters.
pub const DISPLAY_NAME_MAX_LENGTH: usize = 64;

/// `workspaces.allium: config.identity_prefix_length`.
pub const IDENTITY_PREFIX_LENGTH: usize = 8;

/// How a person is named wherever a card or a tool result names them
/// (`workspaces.allium: ShownName`, `CreatorIsShownByName`): their display
/// name, or the identity's short prefix when they have set none. `pub` so the
/// board and `MemoryReducerCaller` call it instead of re-slicing an identity.
pub fn shown_name(display_name: &str, identity: &str) -> String {
    if display_name.is_empty() {
        identity.chars().take(IDENTITY_PREFIX_LENGTH).collect()
    } else {
        display_name.to_string()
    }
}

/// The Unicode format (Cf) characters, plus the line and paragraph separators
/// (Zl, Zp): everything that can hide, reorder or break text without a glyph.
/// A range table rather than a crate, so the wasm build stays dependency-free.
const FORMAT_RANGES: &[(char, char)] = &[
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{2028}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

fn is_format_char(c: char) -> bool {
    FORMAT_RANGES
        .iter()
        .any(|&(first, last)| (first..=last).contains(&c))
}

/// Whether `text` holds a control or invisible formatting character. `pub` so
/// the workspace-name check refuses the same set.
pub fn has_control_or_format_characters(text: &str) -> bool {
    text.chars().any(|c| c.is_control() || is_format_char(c))
}

/// Validate a name for `set_display_name`: trimmed, non-empty, at most
/// [`DISPLAY_NAME_MAX_LENGTH`] characters. `pub` for `MemoryReducerCaller`.
pub fn validate_display_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a display name cannot be empty".into());
    }
    // A name is shown inside other people's cards and tool results, including
    // text an agent reads: a newline or a bidi override could pass as
    // something else. Control and format characters are refused.
    if has_control_or_format_characters(trimmed) {
        return Err(
            "a display name cannot contain control or invisible formatting characters".into(),
        );
    }
    let length = trimmed.chars().count();
    if length > DISPLAY_NAME_MAX_LENGTH {
        return Err(format!(
            "a display name is at most {DISPLAY_NAME_MAX_LENGTH} characters; this one is {length}"
        ));
    }
    Ok(trimmed.to_string())
}

/// `workspaces.allium: RegisterUserProfile`. Idempotent: every reconnect runs it.
pub(crate) fn register_user_profile(ctx: &ReducerContext) {
    let identity = caller_identity(ctx);
    if ctx.db.user_profiles().identity().find(&identity).is_none() {
        ctx.db.user_profiles().insert(UserProfile {
            identity,
            display_name: String::new(),
            is_super_admin: false,
            created_at: now(ctx),
        });
    }
}

/// `workspaces.allium: SetDisplayName`: the caller sets their own name.
#[spacetimedb::reducer]
pub fn set_display_name(ctx: &ReducerContext, name: String) -> Result<(), String> {
    let name = validate_display_name(&name)?;
    let identity = caller_identity(ctx);
    let Some(mut profile) = ctx.db.user_profiles().identity().find(&identity) else {
        return Err("no profile for this identity; reconnect and try again".into());
    };
    profile.display_name = name;
    ctx.db.user_profiles().identity().update(profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Unicode format (Cf) range, plus the line and paragraph separators.
    const REFUSED: &[(char, char)] = &[
        ('\u{00AD}', '\u{00AD}'),
        ('\u{0600}', '\u{0605}'),
        ('\u{061C}', '\u{061C}'),
        ('\u{06DD}', '\u{06DD}'),
        ('\u{070F}', '\u{070F}'),
        ('\u{0890}', '\u{0891}'),
        ('\u{08E2}', '\u{08E2}'),
        ('\u{180E}', '\u{180E}'),
        ('\u{200B}', '\u{200F}'),
        ('\u{2028}', '\u{202E}'),
        ('\u{2060}', '\u{2064}'),
        ('\u{2066}', '\u{206F}'),
        ('\u{FEFF}', '\u{FEFF}'),
        ('\u{FFF9}', '\u{FFFB}'),
        ('\u{110BD}', '\u{110BD}'),
        ('\u{110CD}', '\u{110CD}'),
        ('\u{13430}', '\u{1343F}'),
        ('\u{1BCA0}', '\u{1BCA3}'),
        ('\u{1D173}', '\u{1D17A}'),
        ('\u{E0001}', '\u{E0001}'),
        ('\u{E0020}', '\u{E007F}'),
    ];

    #[test]
    fn every_format_character_and_line_separator_is_refused() {
        for &(first, last) in REFUSED {
            for c in first..=last {
                assert!(
                    has_control_or_format_characters(&format!("Al{c}ice")),
                    "U+{:04X} passed",
                    c as u32
                );
            }
        }
    }

    #[test]
    fn ordinary_text_passes() {
        for name in ["Alice", "Åsa Öberg", "李雷", "Zoë 🙂", "a-b_c"] {
            assert!(!has_control_or_format_characters(name), "{name} refused");
        }
    }
}
