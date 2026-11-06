use crate::ValidationError;
use batter_flags_core::{FlagKey, SwitchState, Toggle, Version};

/// Audit actor, validated and redacted in formatting.
#[derive(Clone)]
pub struct Actor(pub(crate) String);
/// Audit explanation, validated and redacted in formatting.
#[derive(Clone)]
pub struct ChangeReason(pub(crate) String);
macro_rules! audit_text {
    ($name:ident, $max:literal) => {
        impl $name {
            /// Require nonempty text within the byte limit, without control characters.
            pub fn new(value: &str) -> Result<Self, ValidationError> {
                if value.is_empty() || value.len() > $max || value.chars().any(char::is_control) {
                    return Err(ValidationError);
                }
                Ok(Self(value.into()))
            }
            /// Explicit access for trusted audit rendering.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}
audit_text!(Actor, 128);
audit_text!(ChangeReason, 1024);

/// Compare against a row's last change; an absent row has version zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Precondition {
    /// Explicitly authorize replacement regardless of the last version.
    Unconditional,
    /// Reject a changed row, or a revision above the store head.
    UnchangedSince(Version),
}
impl Precondition {
    pub(crate) fn label(self) -> String {
        match self {
            Self::Unconditional => "unconditional".into(),
            Self::UnchangedSince(v) => format!("unchanged_since:{}", v.get()),
        }
    }
}
/// Required audit metadata and concurrency policy, carried by every mutation.
#[derive(Clone, Debug)]
pub struct Change {
    pub(crate) actor: Actor,
    pub(crate) reason: ChangeReason,
    pub(crate) precondition: Precondition,
}
impl Change {
    /// Assemble already validated audit metadata. No defaults silently authorize replay.
    pub fn new(actor: Actor, reason: ChangeReason, precondition: Precondition) -> Self {
        Self {
            actor,
            reason,
            precondition,
        }
    }
}
/// A canonical lower-case variant, `[a-z][a-z0-9_]{0,63}`.
#[derive(Clone, Debug)]
pub struct Variant(pub(crate) String);
impl Variant {
    /// Validate a wire variant before acquiring a connection.
    pub fn new(value: &str) -> Result<Self, ValidationError> {
        FlagKey::new(value).map_err(|_| ValidationError)?;
        Ok(Self(value.into()))
    }
}
/// Explicit declaration for a key never published in a catalog.
/// Each arm fixes a compatible kind, value type and state.
#[derive(Clone, Debug)]
pub enum ExplicitValue {
    /// Release toggle.
    ReleaseToggle(Toggle),
    /// Persistent mode toggle.
    ModeToggle(Toggle),
    /// Release enumeration.
    ReleaseEnum(Variant),
    /// Persistent mode enumeration.
    ModeEnum(Variant),
    /// Kill switch.
    Switch(SwitchState),
}
/// A complete write request. Constructors prevent invalid encoding or missing audit data.
#[derive(Clone, Debug)]
pub struct Write {
    pub(crate) key: FlagKey,
    pub(crate) change: Change,
    pub(crate) value: RequestedValue,
}
#[derive(Clone, Debug)]
pub(crate) enum RequestedValue {
    DeclaredFlag(Variant),
    DeclaredSwitch(SwitchState),
    Explicit(ExplicitValue),
}
impl Write {
    /// Set a declared flag's canonical variant. The store resolves its declaration.
    pub fn set(key: FlagKey, variant: Variant, change: Change) -> Self {
        Self {
            key,
            change,
            value: RequestedValue::DeclaredFlag(variant),
        }
    }
    /// Engage or release a declared kill switch.
    pub fn kill(key: FlagKey, state: SwitchState, change: Change) -> Self {
        Self {
            key,
            change,
            value: RequestedValue::DeclaredSwitch(state),
        }
    }
    /// Write a key without requiring a retained declaration. Fleet compatibility still applies.
    pub fn explicit(key: FlagKey, value: ExplicitValue, change: Change) -> Self {
        Self {
            key,
            change,
            value: RequestedValue::Explicit(value),
        }
    }
}
impl RequestedValue {
    pub(crate) fn state(&self) -> serde_json::Value {
        use batter_flags_core::FlagValue;
        match self {
            Self::DeclaredFlag(v)
            | Self::Explicit(ExplicitValue::ReleaseEnum(v) | ExplicitValue::ModeEnum(v)) => {
                serde_json::json!({"encoding":1,"variant":v.0})
            }
            Self::DeclaredSwitch(v) | Self::Explicit(ExplicitValue::Switch(v)) => {
                serde_json::json!({"encoding":1,"switch":v.as_str()})
            }
            Self::Explicit(ExplicitValue::ReleaseToggle(v) | ExplicitValue::ModeToggle(v)) => {
                serde_json::json!({"encoding":1,"variant":v.variant()})
            }
        }
    }
}
/// Archive or restore an existing key, with the same audit and CAS requirements.
#[derive(Clone, Debug)]
pub struct Archive {
    pub(crate) key: FlagKey,
    pub(crate) change: Change,
}
impl Archive {
    /// Construct a lifecycle change. Select archive or restore on the store.
    pub fn new(key: FlagKey, change: Change) -> Self {
        Self { key, change }
    }
}
