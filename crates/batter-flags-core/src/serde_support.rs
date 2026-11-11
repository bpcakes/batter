//! Serialization of the validated name types as their text.
//!
//! Each deserializes through its validating constructor, so a decoded value
//! satisfies the same grammar as a constructed one.

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::key::{FlagKey, SettingName, SettingsPrefix};

macro_rules! text_serde {
    ($name:ty) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <Cow<'de, str>>::deserialize(deserializer)?;
                <$name>::new(&text).map_err(D::Error::custom)
            }
        }
    };
}

text_serde!(FlagKey);
text_serde!(SettingName);
text_serde!(SettingsPrefix);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_refuse_invalid_text() {
        let key: FlagKey = serde_json::from_str("\"new_checkout\"").unwrap();
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"new_checkout\"");
        assert!(serde_json::from_str::<FlagKey>("\"New\"").is_err());
        assert!(serde_json::from_str::<SettingName>("\"lower\"").is_err());
        assert!(serde_json::from_str::<SettingsPrefix>("\"FLAG\"").is_err());
        let prefix: SettingsPrefix = serde_json::from_str("\"FLAG_\"").unwrap();
        assert_eq!(prefix.as_str(), "FLAG_");
    }
}
