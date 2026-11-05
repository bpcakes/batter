//! The JSON form of a pin.

use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeStruct,
};
use serde_json::{Map, Value};

use super::{PinOrigin, PinRejection, Pinned, Repr, V1};
use crate::{digest::Revision, value::FlagValue, version::Version};

/// The only envelope version this build reads.
const ENVELOPE_VERSION: u64 = 1;

/// A pin this build cannot use, with the JSON it was read from.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct Unusable {
    raw: Value,
    reason: Why,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Why {
    UnsupportedVersion(u64),
    Malformed,
}

impl Unusable {
    pub(crate) fn rejection(&self) -> PinRejection {
        match self.reason {
            Why::UnsupportedVersion(v) => PinRejection::UnsupportedVersion { v },
            Why::Malformed => PinRejection::Malformed,
        }
    }
}

impl Serialize for PinOrigin {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut origin = serializer.serialize_struct("PinOrigin", 2)?;
        origin.serialize_field("class", self.class())?;
        match self {
            Self::Declared { revision } | Self::NotLoaded { revision } => {
                origin.serialize_field("revision", revision)?;
            }
            Self::ControlPlane { version }
            | Self::PersistedFile { version }
            | Self::Test { version } => origin.serialize_field("version", version)?,
        }
        origin.end()
    }
}

impl<T: FlagValue> Serialize for Pinned<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.repr {
            Repr::V1(body) => {
                let mut pin = serializer.serialize_struct("Pinned", 6)?;
                pin.serialize_field("v", &ENVELOPE_VERSION)?;
                pin.serialize_field("key", &*body.key)?;
                pin.serialize_field("kind", &*body.kind)?;
                pin.serialize_field("value_type", &*body.value_type)?;
                pin.serialize_field("variant", &*body.variant)?;
                pin.serialize_field("origin", &body.origin)?;
                pin.end()
            }
            Repr::Unusable(unusable) => unusable.raw.serialize(serializer),
        }
    }
}

impl<'de, T: FlagValue> Deserialize<'de> for Pinned<T> {
    /// Decodes a pin object without failing on what a newer edge may have
    /// written: an envelope version other than 1 and a malformed body decode
    /// into a pin that every replay refuses with a typed
    /// [`PinRejection`]. Only a value that is not a pin envelope at all, an
    /// object with an integer `v`, is a decode error.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let Some(object) = raw.as_object() else {
            return Err(D::Error::custom("a pin is a JSON object"));
        };
        let Some(version) = object.get("v").and_then(Value::as_u64) else {
            return Err(D::Error::custom(
                "a pin carries an integer envelope version v",
            ));
        };
        let repr = if version != ENVELOPE_VERSION {
            Repr::Unusable(Unusable {
                reason: Why::UnsupportedVersion(version),
                raw,
            })
        } else if let Some(body) = decode_body(object) {
            Repr::V1(body)
        } else {
            Repr::Unusable(Unusable {
                reason: Why::Malformed,
                raw,
            })
        };
        Ok(Self::from_repr(repr))
    }
}

/// Version 1: the key, kind, value type and variant as text and a valid
/// origin. Other fields are ignored.
fn decode_body(object: &Map<String, Value>) -> Option<V1> {
    let text = |name: &str| object.get(name)?.as_str().map(Box::from);
    Some(V1 {
        key: text("key")?,
        kind: text("kind")?,
        value_type: text("value_type")?,
        variant: text("variant")?,
        origin: decode_origin(object.get("origin")?.as_object()?)?,
    })
}

/// An origin's class decides which one field it carries: a revision for
/// `declared` and `not_loaded`, a version for the rest. A class with the
/// other class's field, or none, is malformed.
fn decode_origin(origin: &Map<String, Value>) -> Option<PinOrigin> {
    let class = origin.get("class")?.as_str()?;
    let revision = || origin.get("revision")?.as_str()?.parse::<Revision>().ok();
    let version = || origin.get("version")?.as_u64().map(Version::new);
    let (has_revision, has_version) = (
        origin.contains_key("revision"),
        origin.contains_key("version"),
    );
    match class {
        "declared" if !has_version => Some(PinOrigin::Declared {
            revision: revision()?,
        }),
        "not_loaded" if !has_version => Some(PinOrigin::NotLoaded {
            revision: revision()?,
        }),
        "control_plane" if !has_revision => Some(PinOrigin::ControlPlane {
            version: version()?,
        }),
        "persisted_file" if !has_revision => Some(PinOrigin::PersistedFile {
            version: version()?,
        }),
        "test" if !has_revision => Some(PinOrigin::Test {
            version: version()?,
        }),
        _ => None,
    }
}
