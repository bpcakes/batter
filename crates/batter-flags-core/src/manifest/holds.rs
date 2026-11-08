//! The invariant judgement a process makes, replayed over a manifest.

use super::{Manifest, ManifestEntry, ManifestInvariant, ManifestOwners};
use crate::{
    invariants::{Relation, Standing, judge},
    key::FlagKey,
};

/// A flag a process would hold for an unmet invariant, with the invariant
/// that held it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvariantHold {
    /// The flag held: it serves its declared base.
    pub held: FlagKey,
    /// The flag the invariant is declared on.
    pub flag: FlagKey,
    /// The invariant.
    pub invariant: ManifestInvariant,
}

impl Manifest {
    /// The holds a process of `kind` would make for unmet invariants, given
    /// what each flag stands at before invariants are judged: the same
    /// judgement snapshot validation makes, over the flags `kind` owns.
    /// `standing` returns `None` for a kill switch, which no invariant names.
    /// An unknown `kind` owns only the flags every kind owns.
    ///
    /// ```
    /// use batter_flags_core::{CatalogBuilder, SettingsPrefix, Standing, Toggle};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    /// builder.process_kind("serving")?;
    /// builder.environment_class("hosted")?;
    /// let ledger_v2 = builder.mode_flag("ledger_v2", Toggle::Off).declare()?;
    /// let ledger_audit = builder.mode_flag("ledger_audit", Toggle::Off).declare()?;
    /// builder.requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)?;
    /// let manifest = builder.build()?.manifest();
    ///
    /// // `ledger_v2` is published on; `ledger_audit` stands at its default.
    /// let holds = manifest.invariant_holds("serving", |entry| {
    ///     Some(match entry.key().as_str() {
    ///         "ledger_v2" => Standing::Published { variant: "on", base: "off" },
    ///         _ => Standing::Fixed("off"),
    ///     })
    /// });
    /// assert_eq!(holds.len(), 1);
    /// assert_eq!(holds[0].held.as_str(), "ledger_v2");
    /// # Ok(())
    /// # }
    /// ```
    pub fn invariant_holds<'a>(
        &'a self,
        kind: &str,
        mut standing: impl FnMut(&'a ManifestEntry) -> Option<Standing<'a>>,
    ) -> Vec<InvariantHold> {
        let owned = |entry: &ManifestEntry| match entry.owners() {
            ManifestOwners::All => true,
            ManifestOwners::Only(kinds) => kinds.iter().any(|owner| owner == kind),
        };
        let mut nodes: Vec<(&ManifestEntry, Standing<'a>)> = Vec::new();
        for entry in self.flags() {
            if owned(entry)
                && let Some(stands) = standing(entry)
            {
                nodes.push((entry, stands));
            }
        }
        let node_of = |key: &FlagKey| nodes.iter().position(|(entry, _)| entry.key() == key);
        let mut relations = Vec::new();
        let mut stated: Vec<(&ManifestEntry, &ManifestInvariant)> = Vec::new();
        for (flag, (entry, _)) in nodes.iter().enumerate() {
            for invariant in entry.invariants() {
                if let Some(other) = node_of(invariant.other()) {
                    relations.push(Relation {
                        rule: invariant.rule(),
                        flag,
                        variant: invariant.variant(),
                        other,
                        other_variant: invariant.other_variant(),
                    });
                    stated.push((entry, invariant));
                }
            }
        }
        let keys: Vec<_> = nodes.iter().map(|(entry, _)| entry.key()).collect();
        let standings: Vec<_> = nodes.iter().map(|(_, stands)| *stands).collect();
        judge(&keys, &standings, &relations)
            .into_iter()
            .map(|hold| {
                let (declared_on, invariant) = stated[hold.relation];
                InvariantHold {
                    held: keys[hold.flag].clone(),
                    flag: declared_on.key().clone(),
                    invariant: invariant.clone(),
                }
            })
            .collect()
    }
}
