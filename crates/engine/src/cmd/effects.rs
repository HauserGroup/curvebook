//! Side-effect bitflags attached to every [`super::CommandSpec`].

use std::ops::{BitOr, BitOrAssign};

/// Bitmask describing the observable side effects and idempotency of an engine command.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Effects(u16);

impl Effects {
    /// No observable side effects (pure query or read-only inspection).
    pub const NONE: Self = Self(0);
    /// Mutates the in-memory `ProjectDocument`.
    pub const MUTATES_DOCUMENT: Self = Self(1 << 0);
    /// Reads files or directories from the local filesystem.
    pub const READS_FILESYSTEM: Self = Self(1 << 1);
    /// Writes or overwrites files or directories on the local filesystem.
    pub const WRITES_FILESYSTEM: Self = Self(1 << 2);
    /// Launches or interacts with an external helper process.
    pub const LAUNCHES_PROCESS: Self = Self(1 << 3);
    /// Alters UI / navigator selection or presentation state.
    pub const CHANGES_UI_STATE: Self = Self(1 << 4);
    /// May discard unsaved document contents, undo state, or existing data.
    pub const MAY_DISCARD_DATA: Self = Self(1 << 5);
    /// Re-running the command with identical parameters leaves state converged.
    pub const IDEMPOTENT: Self = Self(1 << 6);

    const KNOWN_MASK: u16 = Self::MUTATES_DOCUMENT.0
        | Self::READS_FILESYSTEM.0
        | Self::WRITES_FILESYSTEM.0
        | Self::LAUNCHES_PROCESS.0
        | Self::CHANGES_UI_STATE.0
        | Self::MAY_DISCARD_DATA.0
        | Self::IDEMPOTENT.0;

    /// Construct [`Effects`] from a raw bit representation.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Construct [`Effects`] ignoring any unknown bits.
    #[must_use]
    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::KNOWN_MASK)
    }

    /// Return the underlying bit representation.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Return `true` when no effect flags are set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Combine two effect sets (`self | other`).
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Alias for [`Effects::with`].
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        self.with(other)
    }

    /// Return `true` when every bit in `other` is set in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Return `true` when any bit in `other` is set in `self`.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    /// Return canonical flag names for all enabled bits in deterministic order.
    #[must_use]
    pub fn flag_names(self) -> Vec<&'static str> {
        let mut names = Vec::new();
        let table = [
            (Self::MUTATES_DOCUMENT, "MUTATES_DOCUMENT"),
            (Self::READS_FILESYSTEM, "READS_FILESYSTEM"),
            (Self::WRITES_FILESYSTEM, "WRITES_FILESYSTEM"),
            (Self::LAUNCHES_PROCESS, "LAUNCHES_PROCESS"),
            (Self::CHANGES_UI_STATE, "CHANGES_UI_STATE"),
            (Self::MAY_DISCARD_DATA, "MAY_DISCARD_DATA"),
            (Self::IDEMPOTENT, "IDEMPOTENT"),
        ];
        for (flag, name) in table {
            if self.contains(flag) {
                names.push(name);
            }
        }
        names
    }

    /// Hint for callers (such as MCP tool metadata) whether this command is effectively read-only.
    #[must_use]
    pub const fn read_only_hint(self) -> bool {
        !self.intersects(Self::MUTATES_DOCUMENT.with(Self::WRITES_FILESYSTEM).with(Self::LAUNCHES_PROCESS).with(Self::MAY_DISCARD_DATA))
    }

    /// Hint for callers whether this command may discard or overwrite user data.
    #[must_use]
    pub const fn destructive_hint(self) -> bool {
        self.contains(Self::MAY_DISCARD_DATA)
    }

    /// Return `true` when external I/O, subprocess launch, or destructive effects require user consent.
    #[must_use]
    pub const fn needs_consent(self) -> bool {
        self.intersects(Self::READS_FILESYSTEM.with(Self::WRITES_FILESYSTEM).with(Self::LAUNCHES_PROCESS).with(Self::MAY_DISCARD_DATA))
    }
}

impl BitOr for Effects {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.with(rhs)
    }
}

impl BitOrAssign for Effects {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::Effects;

    #[test]
    fn combinators_and_hints_work_accurately() {
        assert!(Effects::NONE.is_empty());
        assert!(Effects::NONE.read_only_hint());
        assert!(!Effects::NONE.destructive_hint());
        assert!(!Effects::NONE.needs_consent());
        assert!(Effects::NONE.flag_names().is_empty());

        let export = Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT);
        assert!(!export.read_only_hint());
        assert!(!export.destructive_hint());
        assert!(export.needs_consent());
        assert_eq!(export.flag_names(), vec!["WRITES_FILESYSTEM", "IDEMPOTENT"]);

        let mut wipe = Effects::MUTATES_DOCUMENT | Effects::MAY_DISCARD_DATA;
        wipe |= Effects::IDEMPOTENT;
        assert!(wipe.contains(Effects::MUTATES_DOCUMENT));
        assert!(wipe.contains(Effects::MAY_DISCARD_DATA));
        assert!(wipe.destructive_hint());
        assert!(wipe.needs_consent());
        assert!(!wipe.read_only_hint());
        assert_eq!(wipe.flag_names(), vec!["MUTATES_DOCUMENT", "MAY_DISCARD_DATA", "IDEMPOTENT"]);
        assert_eq!(Effects::from_bits_truncate(u16::MAX).bits() & (1 << 15), 0);
    }
}
