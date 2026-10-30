//! Whether an epic or work package is seen by its whole workspace or only by
//! its creator (`docs/specs/workspaces.allium`: `CreateEpicInWorkspace`,
//! `SetEpicVisibility`). The store spells it `"public"` and `"private"`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Every member of the workspace sees it.
    #[default]
    Public,
    /// Only its creator sees it; no admin does.
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Visibility::Public),
            "private" => Some(Visibility::Private),
            _ => None,
        }
    }

    /// The other value, for a form toggle.
    pub fn toggled(self) -> Self {
        match self {
            Visibility::Public => Visibility::Private,
            Visibility::Private => Visibility::Public,
        }
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_round_trips_the_stores_spelling() {
        for v in [Visibility::Public, Visibility::Private] {
            assert_eq!(Visibility::parse(v.as_str()), Some(v));
        }
        assert_eq!(Visibility::parse("secret"), None);
    }

    #[test]
    fn a_toggle_flips_it() {
        assert_eq!(Visibility::Public.toggled(), Visibility::Private);
        assert_eq!(Visibility::Private.toggled(), Visibility::Public);
    }
}
