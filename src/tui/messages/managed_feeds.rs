//! Managed-feed config popup messages.
//!
//! The popup edits the four managed-feed settings (reviews/CVE feed command +
//! optional poll interval). Opening copies the current persisted values into an
//! edit buffer; saving validates, persists, and re-fires provisioning. See
//! `docs/specs/epics.allium` (the `config` block / `ProvisionManagedEpics`).

/// Which field the config popup cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedFeedField {
    ReviewsCommand,
    ReviewsInterval,
    CveCommand,
    CveInterval,
}

impl ManagedFeedField {
    /// Field order for cursor movement, top to bottom.
    pub const ORDER: [ManagedFeedField; 4] = [
        ManagedFeedField::ReviewsCommand,
        ManagedFeedField::ReviewsInterval,
        ManagedFeedField::CveCommand,
        ManagedFeedField::CveInterval,
    ];

    /// True for the numeric interval fields (digit-only input).
    pub fn is_interval(self) -> bool {
        matches!(
            self,
            ManagedFeedField::ReviewsInterval | ManagedFeedField::CveInterval
        )
    }
}

#[derive(Debug, Clone)]
pub enum ManagedFeedConfigMessage {
    /// Open the popup, populating the edit buffer from current settings.
    Open,
    /// Close the popup. `save = true` validates + persists + re-provisions;
    /// `save = false` discards edits.
    Close { save: bool },
    /// Move the field cursor by `delta` (wraps).
    MoveField(isize),
    /// Append a character to the focused field (interval fields accept digits only).
    Input(char),
    /// Delete the last character of the focused field.
    Backspace,
}
