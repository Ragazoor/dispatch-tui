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

/// The Unicode format (Cf) characters that can hide or reorder text: zero-width
/// and bidi controls, the word joiner and the byte-order mark.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
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
    if trimmed.chars().any(|c| c.is_control() || is_format_char(c)) {
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
