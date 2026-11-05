//! What a process may do with a declaration's setting.
//!
//! Declared mode asks two different questions about a declaration, and a
//! mistake between them is silent, so each has a name and the answer is
//! computed once per declaration: whether a value may be *presented* for it in
//! this process, and whether this process *evaluates* it at all. Every check of
//! resolution and the startup report read the same value.

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
    /// The kind does not own the declaration: it never evaluates here, and no
    /// value may be presented.
    NotOwned,
}

impl Applicability {
    pub(crate) fn of(declared: &Declared<'_>, process: Process<'_>) -> Self {
        let owned = match declared.owners() {
            Owners::All => true,
            Owners::Only(kinds) => kinds.contains(process.kind),
        };
        if !owned {
            return Self::NotOwned;
        }
        match declared.only_in() {
            Some(only) if only != process.class => Self::OutsideClass(only.clone()),
            Some(_) | None => Self::Presentable,
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
        !matches!(self, Self::NotOwned)
    }
}
