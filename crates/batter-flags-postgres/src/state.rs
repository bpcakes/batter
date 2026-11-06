//! Decode database wire columns once into explicit row and process lifecycles.
use crate::{
    BlockingBuild, ErrorClass, ExplicitValue, Presence, StoreError, request::RequestedValue,
};
use sqlx::{Row, postgres::PgRow};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Shape {
    ReleaseToggle,
    ReleaseEnum,
    ModeToggle,
    ModeEnum,
    Switch,
}
impl Shape {
    pub(crate) fn read(row: &PgRow) -> Result<Self, StoreError> {
        match (
            row.try_get::<&str, _>("kind")?,
            row.try_get::<&str, _>("value_type")?,
        ) {
            ("release", "toggle") => Ok(Self::ReleaseToggle),
            ("release", "enum") => Ok(Self::ReleaseEnum),
            ("mode", "toggle") => Ok(Self::ModeToggle),
            ("mode", "enum") => Ok(Self::ModeEnum),
            ("switch", "switch") => Ok(Self::Switch),
            _ => Err(StoreError::classified(ErrorClass::InvalidData)),
        }
    }
    pub(crate) fn tags(self) -> (&'static str, &'static str) {
        match self {
            Self::ReleaseToggle => ("release", "toggle"),
            Self::ReleaseEnum => ("release", "enum"),
            Self::ModeToggle => ("mode", "toggle"),
            Self::ModeEnum => ("mode", "enum"),
            Self::Switch => ("switch", "switch"),
        }
    }
    pub(crate) fn accepts(self, request: &RequestedValue) -> bool {
        match request {
            RequestedValue::DeclaredFlag(_) => match self {
                Self::ReleaseToggle | Self::ReleaseEnum | Self::ModeToggle | Self::ModeEnum => true,
                Self::Switch => false,
            },
            RequestedValue::DeclaredSwitch(_) => self == Self::Switch,
            RequestedValue::Explicit(v) => self == Self::explicit(v),
        }
    }
    pub(crate) fn explicit(value: &ExplicitValue) -> Self {
        match value {
            ExplicitValue::ReleaseToggle(_) => Self::ReleaseToggle,
            ExplicitValue::ReleaseEnum(_) => Self::ReleaseEnum,
            ExplicitValue::ModeToggle(_) => Self::ModeToggle,
            ExplicitValue::ModeEnum(_) => Self::ModeEnum,
            ExplicitValue::Switch(_) => Self::Switch,
        }
    }
    pub(crate) fn known_encoding(self, state: &serde_json::Value) -> bool {
        state.as_object().is_some_and(|o| {
            o.len() == 2
                && o.get("encoding").and_then(serde_json::Value::as_u64) == Some(1)
                && match self {
                    Self::Switch => matches!(
                        o.get("switch").and_then(serde_json::Value::as_str),
                        Some("engaged" | "released")
                    ),
                    Self::ReleaseToggle | Self::ReleaseEnum | Self::ModeToggle | Self::ModeEnum => {
                        o.get("variant").is_some_and(serde_json::Value::is_string)
                    }
                }
        })
    }
}
pub(crate) struct ExistingRow {
    pub(crate) shape: Shape,
    pub(crate) state: serde_json::Value,
    pub(crate) version: i64,
}
pub(crate) enum StoredRow {
    Absent,
    Active(ExistingRow),
    Archived(ExistingRow),
}
impl StoredRow {
    pub(crate) fn read(row: Option<PgRow>) -> Result<Self, StoreError> {
        match row {
            None => Ok(Self::Absent),
            Some(row) => {
                let existing = ExistingRow {
                    shape: Shape::read(&row)?,
                    state: row.try_get("state")?,
                    version: row.try_get("updated_version")?,
                };
                match row.try_get::<Option<i64>, _>("archived_version")? {
                    None => Ok(Self::Active(existing)),
                    Some(_) => Ok(Self::Archived(existing)),
                }
            }
        }
    }
    pub(crate) fn existing(&self) -> Option<&ExistingRow> {
        match self {
            Self::Absent => None,
            Self::Active(row) | Self::Archived(row) => Some(row),
        }
    }
    pub(crate) fn version(&self) -> i64 {
        match self {
            Self::Absent => 0, // Wire CAS contract; runtime absence is the enum arm.
            Self::Active(row) | Self::Archived(row) => row.version,
        }
    }
}
pub(crate) enum Process {
    Live(PgRow),
    Silent(PgRow),
    Stopped,
}
impl Process {
    pub(crate) fn read(row: PgRow) -> Result<Self, StoreError> {
        match row.try_get::<&str, _>("presence")? {
            "live" => Ok(Self::Live(row)),
            "silent" => Ok(Self::Silent(row)),
            "stopped" => Ok(Self::Stopped),
            _ => Err(StoreError::classified(ErrorClass::InvalidData)),
        }
    }
    pub(crate) fn active(&self) -> Option<(&PgRow, Presence)> {
        match self {
            Self::Live(row) => Some((row, Presence::Live)),
            Self::Silent(row) => Some((row, Presence::Silent)),
            Self::Stopped => None,
        }
    }
}
pub(crate) fn blocking(row: &PgRow, presence: Presence) -> BlockingBuild {
    BlockingBuild {
        build: row.get("build"),
        presence,
    }
}
