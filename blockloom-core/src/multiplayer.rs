//! Project opt-in for native LAN multiplayer.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MultiplayerSettings {
    pub enabled: bool,
    pub max_guests: usize,
    /// The UDP port `open game to LAN` listens on (0 picks a free one).
    pub port: u16,
    /// Tell the local network about an open game, invite and all, so the
    /// same game elsewhere can list and join it without typing an invite.
    pub announce: bool,
}

impl Default for MultiplayerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_guests: 8,
            port: 7777,
            announce: false,
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
            port: u16,
            announce: bool,
        }
        impl Default for Fields {
            fn default() -> Self {
                let d = MultiplayerSettings::default();
                Self {
                    enabled: d.enabled,
                    max_guests: d.max_guests,
                    port: d.port,
                    announce: d.announce,
                }
            }
        }
        let fields = Fields::deserialize(deserializer)?;
        let settings = Self {
            enabled: fields.enabled,
            max_guests: fields.max_guests,
            port: fields.port,
            announce: fields.announce,
        };
        settings.validate().map_err(serde::de::Error::custom)?;
        Ok(settings)
    }
}
