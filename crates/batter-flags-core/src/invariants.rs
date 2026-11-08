//! The one invariant judgement: which published flags a process holds so
//! that every cross-flag invariant between flags it owns is met.
//!
//! Snapshot validation runs it over what a view would serve, and a writer
//! replays it over a manifest and the stored rows through
//! [`Manifest::invariant_holds`](crate::Manifest::invariant_holds), so the two
//! cannot disagree about which flag an unmet invariant holds.

use crate::{declaration::InvariantRule, key::FlagKey};

/// What a release or mode flag serves before invariants are judged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagStanding<'a> {
    /// A published variant; holding the flag serves `base` in its place.
    Published {
        /// The published variant.
        variant: &'a str,
        /// The declared base a hold serves instead.
        base: &'a str,
    },
    /// A variant no hold changes: the declared base, or the code default a
    /// governing kill switch overrides the flag to.
    Fixed(&'a str),
}

impl<'a> FlagStanding<'a> {
    fn served(self) -> &'a str {
        match self {
            Self::Published { variant, .. } | Self::Fixed(variant) => variant,
        }
    }

    /// What a hold leaves the flag serving; `None` when nothing can be held.
    fn held(self) -> Option<Self> {
        match self {
            Self::Published { base, .. } => Some(Self::Fixed(base)),
            Self::Fixed(_) => None,
        }
    }
}

/// One invariant between two judged flags, by their positions.
pub(crate) struct Relation<'a> {
    pub(crate) rule: InvariantRule,
    pub(crate) flag: usize,
    pub(crate) variant: &'a str,
    pub(crate) other: usize,
    pub(crate) other_variant: &'a str,
}

impl Relation<'_> {
    fn holds(&self, variant: &str, other_variant: &str) -> bool {
        self.rule
            .admits((variant, self.variant), (other_variant, self.other_variant))
    }
}

/// A flag the judgement held, and the relation that held it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Hold {
    pub(crate) flag: usize,
    pub(crate) relation: usize,
}

/// What the judgement decided: the holds it made, and the relations it left
/// unmet because neither flag could be held.
pub(crate) struct Judgement {
    pub(crate) holds: Vec<Hold>,
    #[cfg_attr(
        not(feature = "serde"),
        allow(
            dead_code,
            reason = "only the manifest replay, behind `serde`, reports them; a snapshot over a checked base leaves none"
        )
    )]
    pub(crate) unresolved: Vec<usize>,
}

/// Holds published flags until every relation is met.
///
/// Relations are visited in a canonical order, by the declaring flag's key,
/// the rule, the variant, the other flag's key and its variant, so the
/// outcome depends on what the catalog states and never on the order its
/// declarations were made in. For an unmet relation the flag held is, in
/// this order, the flag it is declared on or the other flag: the first
/// holdable one whose base meets the relation against the other's served
/// variant, else the first holdable one. A hold can leave another relation
/// unmet, so the visit repeats; each round holds at least one more flag, so
/// it ends. A relation no hold can meet, both its flags fixed, is returned
/// as unresolved.
pub(crate) fn judge(
    keys: &[&FlagKey],
    standings: &[FlagStanding<'_>],
    relations: &[Relation<'_>],
) -> Judgement {
    let mut order: Vec<usize> = (0..relations.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&relations[a], &relations[b]);
        (
            keys[a.flag],
            a.rule,
            a.variant,
            keys[a.other],
            a.other_variant,
        )
            .cmp(&(
                keys[b.flag],
                b.rule,
                b.variant,
                keys[b.other],
                b.other_variant,
            ))
    });
    // A hold is the one transition: a published flag becomes fixed at its base.
    let mut standings = standings.to_vec();
    let mut holds = Vec::new();
    loop {
        let before = holds.len();
        for &index in &order {
            let relation = &relations[index];
            if let Some((flag, held)) = unmet(relation, &standings) {
                standings[flag] = held;
                holds.push(Hold {
                    flag,
                    relation: index,
                });
            }
        }
        if holds.len() == before {
            let unresolved = order
                .into_iter()
                .filter(|&index| {
                    let relation = &relations[index];
                    !relation.holds(
                        standings[relation.flag].served(),
                        standings[relation.other].served(),
                    )
                })
                .collect();
            return Judgement { holds, unresolved };
        }
    }
}

/// The flag to hold for `relation`, with what the hold leaves it serving,
/// if the relation is unmet and a hold is possible.
fn unmet<'a>(
    relation: &Relation<'_>,
    standings: &[FlagStanding<'a>],
) -> Option<(usize, FlagStanding<'a>)> {
    let (variant, other_variant) = (
        standings[relation.flag].served(),
        standings[relation.other].served(),
    );
    if relation.holds(variant, other_variant) {
        return None;
    }
    let candidates = [relation.flag, relation.other]
        .map(|index| standings[index].held().map(|held| (index, held)));
    let meets = |&(index, held): &(usize, FlagStanding<'a>)| {
        if index == relation.flag {
            relation.holds(held.served(), other_variant)
        } else {
            relation.holds(variant, held.served())
        }
    };
    candidates
        .into_iter()
        .flatten()
        .find(meets)
        .or_else(|| candidates.into_iter().flatten().next())
}
