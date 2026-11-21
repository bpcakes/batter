//! A catalog as the store keeps it: the manifest and its queryable projection.
use crate::StoreError;
use batter_flags_core::{Kind, Manifest, ManifestEntry};
use batter_sqlx::PgScopedSql;
use serde_json::{Value, json};

/// A manifest prepared once for publication: its digests, its JSON and the
/// rows of `flag_catalog_entries`, so a heartbeat never recomputes them.
#[derive(Clone)]
pub(crate) struct CatalogPublication {
    manifest_digest: [u8; 32],
    manifest: Value,
    entries: Value,
}

impl CatalogPublication {
    /// Prepares `manifest` for publication.
    pub(crate) fn new(manifest: &Manifest) -> Self {
        let manifest_json = serde_json::from_slice(&manifest.to_canonical_json())
            .expect("a canonical manifest is JSON");
        Self {
            manifest_digest: *manifest.digest().as_bytes(),
            manifest: manifest_json,
            entries: Value::Array(manifest.flags().iter().map(entry).collect()),
        }
    }

    /// The identity of the published catalog: the digest of its manifest.
    pub(crate) fn manifest_digest(&self) -> &[u8; 32] {
        &self.manifest_digest
    }

    /// Inserts the catalog and its entries unless they are already stored.
    /// Called only when the heartbeat's key-share read found no row.
    pub(crate) async fn insert(&self, sql: &mut PgScopedSql<'_>) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO flags.flag_catalogs(manifest_digest, manifest) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(&self.manifest_digest[..]).bind(&self.manifest)
            .execute(sql.executor()).await?;
        sqlx::query("INSERT INTO flags.flag_catalog_entries(manifest_digest, key, kind, value_type, variants, expiry, only_in, flip_discipline) \
            SELECT $1, r.key, r.kind, r.value_type, r.variants, r.expiry::date, r.only_in, r.flip_discipline \
            FROM jsonb_to_recordset($2) AS r(key text, kind text, value_type text, variants text[], expiry text, only_in text, flip_discipline text) \
            ON CONFLICT DO NOTHING")
            .bind(&self.manifest_digest[..]).bind(&self.entries)
            .execute(sql.executor()).await?;
        Ok(())
    }
}

impl std::fmt::Debug for CatalogPublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogPublication")
            .field("manifest_digest", &hex(&self.manifest_digest))
            .finish_non_exhaustive()
    }
}

/// One `flag_catalog_entries` row: the columns the store's own queries read.
/// Everything else about a declaration stays in the manifest. Variants are
/// sorted and empty for a kill switch.
fn entry(entry: &ManifestEntry) -> Value {
    let mut variants: Vec<&str> = match entry.kind() {
        Kind::Switch => Vec::new(),
        Kind::Release | Kind::Mode => entry.variants().iter().map(String::as_str).collect(),
    };
    variants.sort_unstable();
    json!({
        "key": entry.key().as_str(),
        "kind": entry.kind().as_str(),
        "value_type": entry.value_type().as_str(),
        "variants": variants,
        "expiry": entry.expiry().map(|date| date.to_string()),
        "only_in": entry.only_in(),
        "flip_discipline": entry.flip_discipline().as_str(),
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
