//! Decode database wire columns once into explicit row lifecycles.
use crate::{ErrorClass, ExplicitValue, StoreError, request::RequestedValue};
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
        Self::from_tags(
            row.try_get::<&str, _>("kind")?,
            row.try_get::<&str, _>("value_type")?,
        )
    }
    pub(crate) fn from_tags(kind: &str, value_type: &str) -> Result<Self, StoreError> {
        match (kind, value_type) {
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
    /// Whether a declared flag's variant lies in this shape's closed
    /// vocabulary. A toggle's is `on` and `off`, fixed by the type and so
    /// judged whether or not any Live or Silent process declares the key; an
    /// enumeration's vocabulary is each declaration's own and is judged
    /// against the fleet. Requests other than a declared flag carry a typed
    /// value and always pass.
    pub(crate) fn closed_vocabulary_admits(self, request: &RequestedValue) -> bool {
        use batter_flags_core::{FlagValue, Toggle};
        match (self, request) {
            (Self::ReleaseToggle | Self::ModeToggle, RequestedValue::DeclaredFlag(variant)) => {
                Toggle::from_variant(&variant.0).is_some()
            }
            (
                Self::ReleaseEnum | Self::ModeEnum | Self::Switch,
                RequestedValue::DeclaredFlag(_),
            )
            | (_, RequestedValue::DeclaredSwitch(_) | RequestedValue::Explicit(_)) => true,
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
