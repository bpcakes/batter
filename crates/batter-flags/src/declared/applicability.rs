//! What a process may do with a declaration's setting.
//!
//! Declared mode asks two different questions about a declaration, and a
//! mistake between them is silent, so each has a name and the answer is
//! computed once per declaration: whether a value may be *presented* for it in
//! this process, and whether this process *evaluates* it at all. A third,
//! whether it *replays* the declaration's pins, decides what the process must
//! record about the variants it could not serve. Every check of resolution and
//! the startup report read the same value.

use batter_flags_core::{Declared, EnvironmentClass, Owners, ProcessKind};

/// Who a process is, for the checks that depend on it.
#[derive(Clone, Copy)]
pub(crate) struct Process<'a> {
    pub(crate) kind: &'a ProcessKind,
    pub(crate) class: &'a EnvironmentClass,
}

/// What a process may do with one declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Applicability {
    /// The kind owns the declaration and, for an `only_in` flag, runs in its
    /// class: a value may be presented, and its absence is judged.
    Presentable,
    /// The kind owns the declaration, but it is restricted to another class:
    /// it evaluates here at its code default, and no value may be presented.
    OutsideClass(EnvironmentClass),
    /// The kind does not own the declaration but is listed in its
    /// `pinned_by`: it never evaluates here and no value may be presented, but
    /// it replays the declaration's pins.
    Pinner(ClassFit),
    /// The kind neither owns the declaration nor replays its pins: it never
    /// evaluates here, and no value may be presented.
    NotOwned,
}

/// Where a pin-replaying kind runs against an `only_in` restriction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClassFit {
    /// The declaration is unrestricted, or restricted to this class.
    Inside,
    /// The declaration is restricted to another class.
    Outside,
}

impl Applicability {
    pub(crate) fn of(declared: &Declared<'_>, process: Process<'_>) -> Self {
        let owned = match declared.owners() {
            Owners::All => true,
            Owners::Only(kinds) => kinds.contains(process.kind),
        };
        let outside = declared.only_in().filter(|only| *only != process.class);
        match (owned, outside) {
            (true, Some(only)) => Self::OutsideClass(only.clone()),
            (true, None) => Self::Presentable,
            (false, _) if !declared.pinned_by().contains(process.kind) => Self::NotOwned,
            (false, Some(_)) => Self::Pinner(ClassFit::Outside),
            (false, None) => Self::Pinner(ClassFit::Inside),
        }
    }

    /// Whether a value may be presented for the declaration in this process.
    pub(crate) fn may_present(&self) -> bool {
        matches!(self, Self::Presentable)
    }

    /// Whether this process evaluates the declaration at all: a flag it does
    /// not own is never evaluated here, and one restricted to another class is
    /// evaluated at its code default.
    pub(crate) fn evaluates(&self) -> bool {
        matches!(self, Self::Presentable | Self::OutsideClass(_))
    }

    /// Whether this process replays the declaration's pins: an owner, or a
    /// kind its `pinned_by` lists. What such a process could not serve is
    /// recorded, so a pin selecting it is refused.
    pub(crate) fn replays_pins(&self) -> bool {
        !matches!(self, Self::NotOwned)
    }

    /// Whether the declaration is restricted to a class this process runs
    /// outside, where it serves only the code default.
    pub(crate) fn outside_class(&self) -> bool {
        matches!(
            self,
            Self::OutsideClass(_) | Self::Pinner(ClassFit::Outside)
        )
    }
}
