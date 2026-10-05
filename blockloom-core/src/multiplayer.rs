//! Project opt-in for native LAN spectators.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MultiplayerSettings {
    pub enabled: bool,
    pub max_guests: usize,
}

impl Default for MultiplayerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_guests: 8,
        }
    }
}

impl MultiplayerSettings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.max_guests) {
            return Err("The LAN guest limit must be between 1 and 16".into());
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for MultiplayerSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Fields {
            enabled: bool,
            max_guests: usize,
        }
        impl Default for Fields {
            fn default() -> Self {
                Self {
                    enabled: false,
                    max_guests: 8,
                }
            }
        }
        let fields = Fields::deserialize(deserializer)?;
        let settings = Self {
            enabled: fields.enabled,
            max_guests: fields.max_guests,
        };
        settings.validate().map_err(serde::de::Error::custom)?;
        Ok(settings)
    }
}
