//! A key's retained declarations, the processes that declare it and the
//! catalogs those processes run, read by one statement.
//!
//! Heartbeats do not take the counter lock, so under READ COMMITTED each
//! statement of a write sees its own snapshot: a build that publishes between
//! two reads could be judged by one and missed by the other. One statement
//! sees one snapshot and one `statement_timestamp()`, so the version the write
//! is numbered with, the flip policy, the decoding and archive gates, the
//! invariant judgement and the declarer count all judge the same fleet.

use crate::{BlockingBuild, ErrorClass, Presence, StoreError, state::Shape};
use batter_sqlx::PgScopedSql;
use sqlx::Row;

/// One retained catalog's entry for the key.
pub(crate) struct Declaration {
    kind: String,
    value_type: String,
    /// The entry names the environments it is restricted to.
    pub(crate) restricted: bool,
    /// The entry's flip discipline is coordinated.
    pub(crate) coordinated: bool,
}
impl Declaration {
    pub(crate) fn shape(&self) -> Result<Shape, StoreError> {
        Shape::from_tags(&self.kind, &self.value_type)
    }
}

/// A Live or Silent process, with what its catalog declares for the key.
pub(crate) struct Declarer {
    build: String,
    pub(crate) presence: Presence,
    pub(crate) row_encoding: i16,
    pub(crate) kind: String,
    pub(crate) value_type: String,
    pub(crate) variants: Vec<String>,
}
impl Declarer {
    pub(crate) fn shape(&self) -> Result<Shape, StoreError> {
        Shape::from_tags(&self.kind, &self.value_type)
    }

    pub(crate) fn blocking(&self) -> BlockingBuild {
        BlockingBuild {
            build: self.build.clone(),
            presence: self.presence,
        }
    }
}

/// A process whose catalog declares the key.
pub(crate) enum Process {
    Active(Declarer),
    Stopped,
}

pub(crate) struct Declared {
    /// The highest version a Live control-plane process reports above the
    /// head and inside the jump window, fleet-wide.
    pub(crate) ahead: Option<i64>,
    /// One per catalog, the most recently referenced first.
    pub(crate) declarations: Vec<Declaration>,
    /// Every declaring process row, by build, then process.
    pub(crate) processes: Vec<Process>,
    /// The manifest of each declaring catalog a Live or Silent process runs,
    /// by digest.
    pub(crate) manifests: Vec<serde_json::Value>,
}
impl Declared {
    pub(crate) fn active(&self) -> impl Iterator<Item = &Declarer> {
        self.processes.iter().filter_map(|process| match process {
            Process::Active(declarer) => Some(declarer),
            Process::Stopped => None,
        })
    }

    /// The running declarers' shape for the key, a Live one's first; `None`
    /// when no process that has not stopped declares it.
    pub(crate) fn running_shape(&self) -> Option<Result<Shape, StoreError>> {
        self.active()
            .min_by_key(|declarer| declarer.presence != Presence::Live)
            .map(Declarer::shape)
    }

    /// Reads the key's declarations, with the fleet's highest report above
    /// `head` inside the jump window.
    pub(crate) async fn read(
        sql: &mut PgScopedSql<'_>,
        key: &str,
        head: i64,
    ) -> Result<Self, StoreError> {
        // The fleet row always exists; beside it, one row per catalog entry
        // and declaring process, or one with no process. The manifest rides on
        // the first row of a catalog only when a process that has not stopped
        // runs it.
        let rows = sqlx::query(
            "WITH fleet AS ( \
                SELECT max(snapshot_version) AS ahead FROM flags.flag_processes \
                WHERE stopped_at IS NULL AND expires_at > statement_timestamp() \
                    AND snapshot_source = 'control_plane' AND snapshot_version > $2 \
                    AND snapshot_version - $2 < $3 \
             ), declared AS ( \
             SELECT e.manifest_digest, c.last_referenced_at, e.kind, e.value_type, \
                e.only_in IS NOT NULL AS restricted, \
                e.flip_discipline = 'coordinated' AS coordinated, e.variants, \
                p.process_id IS NOT NULL AS declaring, p.build, p.row_encoding, \
                row_number() OVER (ORDER BY p.build, p.process_id) AS process_order, \
                CASE WHEN p.process_id IS NULL THEN NULL WHEN p.stopped_at IS NOT NULL THEN 'stopped' \
                    WHEN p.expires_at > statement_timestamp() THEN 'live' ELSE 'silent' END AS presence, \
                CASE WHEN row_number() OVER catalog = 1 \
                    AND bool_or(p.process_id IS NOT NULL AND p.stopped_at IS NULL) OVER catalog \
                    THEN c.manifest END AS manifest \
             FROM flags.flag_catalog_entries e \
             JOIN flags.flag_catalogs c ON c.manifest_digest = e.manifest_digest \
             LEFT JOIN flags.flag_processes p ON p.manifest_digest = e.manifest_digest \
             WHERE e.key = $1 \
             WINDOW catalog AS (PARTITION BY e.manifest_digest) \
             ) \
             SELECT fleet.ahead, d.* FROM fleet LEFT JOIN declared d ON true \
             ORDER BY d.last_referenced_at DESC, d.manifest_digest",
        )
        .bind(key)
        .bind(head)
        .bind(crate::MAX_VERSION_JUMP)
        .fetch_all(sql.executor())
        .await?;
        let mut declarations = Vec::new();
        let mut processes = Vec::new();
        let mut manifests = Vec::new();
        let mut catalog: Option<Vec<u8>> = None;
        let mut ahead = None;
        for row in rows {
            ahead = row.try_get("ahead")?;
            let Some(digest) = row.try_get::<Option<Vec<u8>>, _>("manifest_digest")? else {
                continue;
            };
            if catalog.as_ref() != Some(&digest) {
                declarations.push(Declaration {
                    kind: row.try_get("kind")?,
                    value_type: row.try_get("value_type")?,
                    restricted: row.try_get("restricted")?,
                    coordinated: row.try_get("coordinated")?,
                });
                catalog = Some(digest.clone());
            }
            if let Some(manifest) = row.try_get::<Option<serde_json::Value>, _>("manifest")? {
                manifests.push((digest, manifest));
            }
            if !row.try_get::<bool, _>("declaring")? {
                continue;
            }
            let order: i64 = row.try_get("process_order")?;
            let presence = match row.try_get::<&str, _>("presence")? {
                "live" => Presence::Live,
                "silent" => Presence::Silent,
                "stopped" => {
                    processes.push((order, Process::Stopped));
                    continue;
                }
                _ => return Err(StoreError::classified(ErrorClass::InvalidData)),
            };
            processes.push((
                order,
                Process::Active(Declarer {
                    build: row.try_get("build")?,
                    presence,
                    row_encoding: row.try_get("row_encoding")?,
                    kind: row.try_get("kind")?,
                    value_type: row.try_get("value_type")?,
                    variants: row.try_get("variants")?,
                }),
            ));
        }
        // The database orders builds by its own collation.
        processes.sort_by_key(|(order, _)| *order);
        manifests.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Self {
            ahead,
            declarations,
            processes: processes.into_iter().map(|(_, p)| p).collect(),
            manifests: manifests.into_iter().map(|(_, m)| m).collect(),
        })
    }
}
