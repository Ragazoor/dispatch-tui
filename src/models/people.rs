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

/// `core.allium: WorkspaceRole`: fixed, two of them (`workspaces.allium`, D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRole {
    Admin,
    Member,
}

impl MemberRole {
    /// The store's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            MemberRole::Admin => "admin",
            MemberRole::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(MemberRole::Admin),
            "member" => Some(MemberRole::Member),
            _ => None,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            MemberRole::Admin => MemberRole::Member,
            MemberRole::Member => MemberRole::Admin,
        }
    }
}

/// One person in the directory (`admin-surfaces.allium: DirectoryEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPerson {
    pub identity: String,
    pub shown_name: String,
    pub is_super_admin: bool,
}

impl DirectoryPerson {
    /// `"<shown name> (<identity prefix>)"`: how a person is listed when a
    /// name alone would not tell two of them apart.
    pub fn labelled(&self) -> String {
        let prefix: String = self.identity.chars().take(IDENTITY_PREFIX_LENGTH).collect();
        format!("{} ({prefix})", self.shown_name)
    }
}

/// One person's membership of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMember {
    pub membership_id: i64,
    pub identity: String,
    pub shown_name: String,
    pub role: MemberRole,
}

/// A workspace with its members, ordered by shown name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDetail {
    pub id: i64,
    pub name: String,
    pub members: Vec<WorkspaceMember>,
}

/// One row of the unseen-item listing (`admin-surfaces.allium: UnseenItem`).
/// Never a title or any content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnseenItemRow {
    /// `"epic"` or `"work_package"`.
    pub kind: String,
    pub id: i64,
    pub workspace_id: i64,
    pub workspace_name: String,
    pub creator: String,
    pub created_at: String,
}

/// Everything the admin screen and the list tools show, as the board last
/// read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminSnapshot {
    /// The identity this board writes as; `None` until it settles.
    pub me: Option<String>,
    pub i_am_super_admin: bool,
    /// No super-admin exists, so `claim_super_admin` would work.
    pub claimable: bool,
    /// Every workspace the person administers or belongs to: all of them for
    /// a super-admin.
    pub workspaces: Vec<WorkspaceDetail>,
    pub directory: Vec<DirectoryPerson>,
    pub unseen: Vec<UnseenItemRow>,
}

impl AdminSnapshot {
    /// Whether the person may rename, add to or remove from `workspace`.
    pub fn administers(&self, workspace: &WorkspaceDetail) -> bool {
        self.i_am_super_admin
            || self.me.as_ref().is_some_and(|me| {
                workspace
                    .members
                    .iter()
                    .any(|m| &m.identity == me && m.role == MemberRole::Admin)
            })
    }

    pub fn super_admins(&self) -> impl Iterator<Item = &DirectoryPerson> {
        self.directory.iter().filter(|p| p.is_super_admin)
    }
}

/// `admin-surfaces.allium: PersonResolution`: find one person by display name
/// or identity. An identity (whole, or a prefix of at least
/// `IDENTITY_PREFIX_LENGTH` characters) wins over a name. A name that more
/// than one person has is refused, listing them, rather than resolved to one.
pub fn resolve_person<'a>(
    directory: &'a [DirectoryPerson],
    text: &str,
) -> Result<&'a DirectoryPerson, String> {
    let wanted = text.trim();
    if wanted.is_empty() {
        return Err("name a person by display name or identity".to_string());
    }
    let by_identity: Vec<&DirectoryPerson> = directory
        .iter()
        .filter(|p| {
            p.identity == wanted
                || (wanted.chars().count() >= IDENTITY_PREFIX_LENGTH
                    && p.identity.starts_with(wanted))
        })
        .collect();
    let by_name: Vec<&DirectoryPerson> = directory
        .iter()
        .filter(|p| p.shown_name == wanted)
        .collect();
    let matches = if by_identity.is_empty() {
        by_name
    } else {
        by_identity
    };
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(format!(
            "no one is called '{wanted}'; list_workspaces shows the directory of people"
        )),
        many => Err(format!(
            "more than one person matches '{wanted}': {}. Name one by identity",
            many.iter()
                .map(|p| p.labelled())
                .collect::<Vec<_>>()
                .join(", ")
        )),
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

    fn person(identity: &str, name: &str) -> DirectoryPerson {
        DirectoryPerson {
            identity: identity.to_string(),
            shown_name: name.to_string(),
            is_super_admin: false,
        }
    }

    #[test]
    fn a_unique_name_resolves_to_its_person() {
        let dir = [person("aaaaaaaa11", "Ann"), person("bbbbbbbb22", "Bob")];
        assert_eq!(
            resolve_person(&dir, " Bob ").unwrap().identity,
            "bbbbbbbb22"
        );
    }

    #[test]
    fn a_duplicated_name_is_refused_listing_each_match() {
        let dir = [person("aaaaaaaa11", "Ann"), person("bbbbbbbb22", "Ann")];
        let why = resolve_person(&dir, "Ann").unwrap_err();
        assert!(
            why.contains("Ann (aaaaaaaa)") && why.contains("Ann (bbbbbbbb)"),
            "{why}"
        );
    }

    #[test]
    fn an_identity_tells_a_duplicated_name_apart() {
        let dir = [person("aaaaaaaa11", "Ann"), person("bbbbbbbb22", "Ann")];
        assert_eq!(
            resolve_person(&dir, "bbbbbbbb").unwrap().identity,
            "bbbbbbbb22"
        );
    }

    #[test]
    fn a_short_identity_prefix_is_not_an_identity() {
        let dir = [person("aaaaaaaa11", "Ann")];
        assert!(resolve_person(&dir, "aaaa").is_err());
    }

    #[test]
    fn an_identity_match_wins_over_a_name_that_looks_like_it() {
        let dir = [
            person("aaaaaaaa11", "Ann"),
            person("bbbbbbbb22", "aaaaaaaa"),
        ];
        assert_eq!(
            resolve_person(&dir, "aaaaaaaa").unwrap().identity,
            "aaaaaaaa11"
        );
    }

    #[test]
    fn an_unknown_or_blank_name_is_refused() {
        let dir = [person("aaaaaaaa11", "Ann")];
        assert!(resolve_person(&dir, "Zed").is_err());
        assert!(resolve_person(&dir, "  ").is_err());
    }

    #[test]
    fn an_identity_shorter_than_the_prefix_is_shown_whole() {
        assert_eq!(shown_name("", "abc"), "abc");
    }
}
