//! What the catalogs of running declarers make of a write: which flags the
//! processes would newly hold for unmet cross-flag invariants, and which
//! engaged kill switches override the written flag. Judged in the write's
//! own transaction, after the counter lock, against the stored rows.
//!
//! The judgement replays the one a process makes,
//! [`Manifest::invariant_holds`], for every process kind of every catalog a
//! Live or Silent process declares the key in, once over the stored rows and
//! once with the write applied. What it cannot see is disclosed: a process's
//! settings (its declared base, its settings floor and which required
//! settings it has), and staleness, which a stale rule turns into an engaged
//! switch. Each process judges again when it loads and holds what is unmet
//! there; this judgement is the advance notice, not the enforcement.

use std::collections::{BTreeMap, BTreeSet};

use batter_flags_core::{
    FlagKey, FlagStanding, InvariantHold, InvariantRule, Manifest, ManifestEntry, SafeState,
    SwitchState,
};
use batter_sqlx::PgScopedSql;
use sqlx::Row;

use crate::StoreError;

/// What the flags of an unmet invariant were judged by.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Basis {
    /// Both flags by their stored rows or the write itself.
    StoredRows,
    /// At least one flag by its code default: no stored row of it is
    /// served, or an engaged governing switch overrides it. A process serves
    /// its declared base there, which its settings can set otherwise.
    CodeDefault,
}

/// A flag the write would newly hold in processes whose kind owns both flags
/// of an invariant the write leaves unmet; the held flag serves its declared
/// base until the invariant is met.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnmetInvariant {
    /// The flag held.
    pub held: FlagKey,
    /// The relation.
    pub rule: InvariantRule,
    /// The flag the invariant is declared on.
    pub flag: FlagKey,
    /// The variant of `flag` the invariant speaks about.
    pub variant: String,
    /// The other flag.
    pub other: FlagKey,
    /// The variant of `other` the invariant speaks about.
    pub other_variant: String,
    /// What the two flags were judged by.
    pub basis: Basis,
}

/// Why processes will not serve a write as written for now, judged in the
/// write's transaction. Empty when nothing stored stands in its way.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Deferral {
    /// The flags the write would newly hold, sorted. The written flag is
    /// listed whenever it would be held, even if it already was.
    pub unmet: Vec<UnmetInvariant>,
    /// The kill switches that govern the written flag and are engaged by
    /// their stored row or their safe state, sorted: processes serve the
    /// flag's code default until each is released.
    pub overridden_by: Vec<FlagKey>,
    /// Catalogs of Live or Silent declarers whose manifest this writer cannot
    /// decode, a newer build's, so nothing of theirs was judged.
    pub unjudged_catalogs: usize,
    /// Whether a retained catalog declares the written key's flips
    /// coordinated. A policy that accepts it, as [`PooledStore::write`],
    /// `archive` and `restore` do, writes the flip anyway, and processes apply
    /// it poll by poll, not together.
    ///
    /// [`PooledStore::write`]: crate::PooledStore::write
    pub flip: crate::Coordination,
}

/// The state a write leaves its key in, as judged.
#[derive(Clone, Copy)]
pub(crate) enum Written<'a> {
    /// A release or mode flag at this variant.
    Flag(&'a str),
    /// A kill switch in this state.
    Switch(SwitchState),
}

/// The most governing switches with rows readers hold that one catalog's
/// judgement enumerates; a catalog with more counts as unjudged.
const MAX_HELD_GOVERNORS: usize = 4;

/// Judges `written`, the state a write leaves `key` in, against the
/// catalogs of Live and Silent processes that declare `key`.
pub(crate) async fn judge(
    sql: &mut PgScopedSql<'_>,
    key: &FlagKey,
    written: Written<'_>,
) -> Result<Deferral, StoreError> {
    let manifests: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT c.manifest FROM flags.flag_catalogs c \
         WHERE EXISTS (SELECT FROM flags.flag_catalog_entries e WHERE e.manifest_digest = c.manifest_digest AND e.key = $1) \
           AND EXISTS (SELECT FROM flags.flag_processes p WHERE p.manifest_digest = c.manifest_digest AND p.stopped_at IS NULL) \
         ORDER BY c.manifest_digest",
    )
    .bind(key.as_str())
    .fetch_all(sql.executor())
    .await?;
    let mut deferral = Deferral::default();
    let manifests: Vec<Manifest> = manifests
        .into_iter()
        .filter_map(|manifest| {
            let decoded = serde_json::from_value(manifest).ok();
            if decoded.is_none() {
                deferral.unjudged_catalogs += 1;
            }
            decoded
        })
        .collect();
    let names: BTreeSet<String> = manifests
        .iter()
        .flat_map(|manifest| manifest.flags().iter().map(|e| e.key().as_str().to_owned()))
        .collect();
    let rows = Rows(stored(sql, names.into_iter().collect()).await?);
    for manifest in &manifests {
        // A governing switch whose row readers hold serves whatever state each
        // reader last published, independently of every other, so each
        // combination is judged.
        let held: BTreeSet<&FlagKey> = manifest
            .flags()
            .iter()
            .filter_map(ManifestEntry::governed_by)
            .filter(|switch| {
                manifest
                    .flag(switch.as_str())
                    .is_some_and(|entry| matches!(rows.read(entry), Read::Undecodable))
            })
            .collect();
        let held: Vec<&FlagKey> = held.into_iter().collect();
        if held.len() > MAX_HELD_GOVERNORS {
            deferral.unjudged_catalogs += 1;
            continue;
        }
        for subset in 0..1_usize << held.len() {
            let engaged: BTreeSet<&FlagKey> = held
                .iter()
                .enumerate()
                .filter(|(bit, _)| subset & (1 << bit) != 0)
                .map(|(_, switch)| *switch)
                .collect();
            judge_catalog(manifest, &rows, key, written, &engaged, &mut deferral);
        }
    }
    deferral.unmet.sort();
    deferral.unmet.dedup();
    deferral.overridden_by.sort();
    deferral.overridden_by.dedup();
    Ok(deferral)
}

/// Adds to `deferral` what one catalog's processes would make of the write,
/// with the held governing switches in `engaged` taken to serve engaged and
/// every other held one released.
fn judge_catalog(
    manifest: &Manifest,
    rows: &Rows,
    key: &FlagKey,
    written: Written<'_>,
    engaged: &BTreeSet<&FlagKey>,
    deferral: &mut Deferral,
) {
    for kind in manifest.process_kinds() {
        let before = Judged::new(manifest, rows, None, engaged);
        let held_before: BTreeSet<FlagKey> = manifest
            .invariant_holds(kind, |entry| before.standing(entry))
            .holds
            .into_iter()
            .map(|hold| hold.held)
            .collect();
        let after = Judged::new(manifest, rows, Some((key, written)), engaged);
        // Every fixed standing here is a code default, so nothing is left
        // unresolved.
        for hold in manifest
            .invariant_holds(kind, |entry| after.standing(entry))
            .holds
        {
            if hold.held == *key || !held_before.contains(&hold.held) {
                deferral.unmet.push(after.unmet(hold));
            }
        }
    }
    if let (Written::Flag(variant), Some(entry)) = (written, manifest.flag(key.as_str()))
        && let Some(switch) = entry.governed_by()
        && entry.default_variant() != Some(variant)
        && Judged::new(manifest, rows, None, engaged).engaged(switch)
    {
        deferral.overridden_by.push(switch.clone());
    }
}

/// The stored, unarchived rows of the judged keys, by name.
struct Rows(BTreeMap<String, StoredValue>);

struct StoredValue {
    kind: String,
    value_type: String,
    state: serde_json::Value,
}

impl Rows {
    /// The row of `entry`'s key, decoded as a reader decodes it: its kind
    /// and value type the declaration's, state encoding 1 exactly, and a
    /// variant the declaration declares or a switch state.
    fn read(&self, entry: &ManifestEntry) -> Read<'_> {
        let Some(row) = self.0.get(entry.key().as_str()) else {
            return Read::Absent;
        };
        Self::decode(row, entry).unwrap_or(Read::Undecodable)
    }

    fn decode<'r>(row: &'r StoredValue, entry: &ManifestEntry) -> Option<Read<'r>> {
        if row.kind != entry.kind().as_str() || row.value_type != entry.value_type().as_str() {
            return None;
        }
        let fields = row.state.as_object()?;
        if fields.len() != 2 || fields.get("encoding")?.as_u64() != Some(1) {
            return None;
        }
        if let Some(variant) = fields.get("variant") {
            let variant = variant.as_str()?;
            return entry
                .variants()
                .iter()
                .any(|declared| declared == variant)
                .then_some(Read::Flag(variant));
        }
        match fields.get("switch")?.as_str()? {
            "engaged" => Some(Read::Switch(SwitchState::Engaged)),
            "released" => Some(Read::Switch(SwitchState::Released)),
            _ => None,
        }
    }
}

/// What a reader makes of one stored row.
enum Read<'a> {
    /// No unarchived row.
    Absent,
    /// A row a reader holds: another kind or type, another encoding, or an
    /// undeclared variant.
    Undecodable,
    Flag(&'a str),
    Switch(SwitchState),
}

/// The stored rows of one catalog, with a proposed write applied or not.
struct Judged<'a> {
    manifest: &'a Manifest,
    rows: &'a Rows,
    written: Option<(&'a FlagKey, Written<'a>)>,
    /// The governing switches whose rows readers hold that are taken to
    /// serve engaged: a reader keeps whichever state it last published,
    /// which only it knows.
    held_engaged: &'a BTreeSet<&'a FlagKey>,
}

impl<'a> Judged<'a> {
    fn new(
        manifest: &'a Manifest,
        rows: &'a Rows,
        written: Option<(&'a FlagKey, Written<'a>)>,
        held_engaged: &'a BTreeSet<&'a FlagKey>,
    ) -> Self {
        Self {
            manifest,
            rows,
            written,
            held_engaged,
        }
    }

    /// What `entry` serves before invariants, and whether a row or the
    /// write supplied it. `None` for a kill switch.
    fn judged(&self, entry: &'a ManifestEntry) -> Option<(FlagStanding<'a>, Basis)> {
        let default = entry.default_variant()?;
        if let Some(switch) = entry.governed_by()
            && self.engaged(switch)
        {
            return Some((FlagStanding::Fixed(default), Basis::CodeDefault));
        }
        let served = match self.written {
            Some((key, Written::Flag(variant))) if key == entry.key() => Some(variant),
            Some((key, _)) if key == entry.key() => None,
            Some(_) | None => match self.rows.read(entry) {
                Read::Flag(variant) => Some(variant),
                Read::Switch(_) | Read::Absent | Read::Undecodable => None,
            },
        };
        let served = served.filter(|variant| {
            entry.only_in().is_none() && entry.variants().iter().any(|declared| declared == variant)
        });
        Some(match served {
            Some(variant) => (
                FlagStanding::Published {
                    variant,
                    base: default,
                },
                Basis::StoredRows,
            ),
            None => (FlagStanding::Fixed(default), Basis::CodeDefault),
        })
    }

    fn standing(&self, entry: &'a ManifestEntry) -> Option<FlagStanding<'a>> {
        self.judged(entry).map(|(standing, _)| standing)
    }

    /// Whether the kill switch `key` is engaged: by the write, by its stored
    /// row, else by its safe state.
    fn engaged(&self, key: &FlagKey) -> bool {
        if let Some((written, Written::Switch(state))) = self.written
            && written == key
        {
            return state.is_engaged();
        }
        let Some(entry) = self.manifest.flag(key.as_str()) else {
            return false;
        };
        match self.rows.read(entry) {
            Read::Switch(state) => state.is_engaged(),
            Read::Undecodable => self.held_engaged.contains(key),
            Read::Flag(_) | Read::Absent => entry.safe_state() == Some(SafeState::Engaged),
        }
    }

    fn unmet(&self, hold: InvariantHold) -> UnmetInvariant {
        let basis = [&hold.flag, hold.invariant.other()]
            .into_iter()
            .filter_map(|key| self.manifest.flag(key.as_str()))
            .filter_map(|entry| self.judged(entry).map(|(_, basis)| basis))
            .max()
            .unwrap_or(Basis::CodeDefault);
        UnmetInvariant {
            held: hold.held,
            rule: hold.invariant.rule(),
            flag: hold.flag,
            variant: hold.invariant.variant().to_owned(),
            other: hold.invariant.other().clone(),
            other_variant: hold.invariant.other_variant().to_owned(),
            basis,
        }
    }
}

async fn stored(
    sql: &mut PgScopedSql<'_>,
    names: Vec<String>,
) -> Result<BTreeMap<String, StoredValue>, StoreError> {
    if names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = sqlx::query(
        "SELECT name, kind, value_type, state FROM flags.flag_state \
         WHERE name = ANY($1) AND archived_version IS NULL",
    )
    .bind(names)
    .fetch_all(sql.executor())
    .await?;
    let mut stored = BTreeMap::new();
    for row in rows {
        stored.insert(
            row.try_get("name")?,
            StoredValue {
                kind: row.try_get("kind")?,
                value_type: row.try_get("value_type")?,
                state: row.try_get("state")?,
            },
        );
    }
    Ok(stored)
}

/// The state a mutation leaves a key in, as judged; `None` for an archive.
pub(crate) fn written(state: &serde_json::Value) -> Option<Written<'_>> {
    if let Some(variant) = state.get("variant").and_then(serde_json::Value::as_str) {
        return Some(Written::Flag(variant));
    }
    match state.get("switch").and_then(serde_json::Value::as_str)? {
        "engaged" => Some(Written::Switch(SwitchState::Engaged)),
        "released" => Some(Written::Switch(SwitchState::Released)),
        _ => None,
    }
}
