use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ValheimSettings {
    pub world: String,
    pub password: String,
    pub public: bool,
    pub crossplay: bool,
    pub save_interval: u32,
    pub backups: u16,
    pub backup_short: u32,
    pub backup_long: u32,
    pub preset: String,
    pub modifiers: BTreeMap<String, String>,
    pub keys: Vec<String>,
}

impl Default for ValheimSettings {
    fn default() -> Self {
        Self {
            world: "Dedicated".into(),
            password: String::new(),
            public: false,
            crossplay: false,
            save_interval: 1800,
            backups: 4,
            backup_short: 7200,
            backup_long: 43200,
            preset: String::new(),
            modifiers: BTreeMap::new(),
            keys: Vec::new(),
        }
    }
}

impl ValheimSettings {
    pub fn validate(&self, name: &str, creating: bool) -> Result<(), String> {
        if self.world.is_empty()
            || self.world.len() > 80
            || self.world.starts_with('.')
            || self
                .world
                .chars()
                .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        {
            return Err("World name must be 1–80 characters without path separators".into());
        }
        if !(creating && self.password.is_empty())
            && (self.password.chars().count() < 5
                || self.password.len() > 128
                || self.password.chars().any(char::is_control)
                || name.to_lowercase().contains(&self.password.to_lowercase()))
        {
            return Err(
                "Use a password of 5–128 characters that is not part of the server name".into(),
            );
        }
        if !(60..=86400).contains(&self.save_interval)
            || !(1..=100).contains(&self.backups)
            || !(300..=604800).contains(&self.backup_short)
            || !(self.backup_short..=2592000).contains(&self.backup_long)
        {
            return Err("Use saves every 60–86400 seconds, 1–100 backups, a short interval of 300–604800 seconds, and a long interval between the short interval and 30 days".into());
        }
        if ![
            "",
            "normal",
            "casual",
            "easy",
            "hard",
            "hardcore",
            "immersive",
            "hammer",
        ]
        .contains(&self.preset.as_str())
        {
            return Err("Unknown Valheim world preset".into());
        }
        for (key, value) in &self.modifiers {
            let values: &[&str] = match key.as_str() {
                "combat" => &["veryeasy", "easy", "hard", "veryhard"],
                "deathpenalty" => &["casual", "veryeasy", "easy", "hard", "hardcore"],
                "resources" => &["muchless", "less", "more", "muchmore", "most"],
                "raids" => &["none", "muchless", "less", "more", "muchmore"],
                "portals" => &["casual", "hard", "veryhard"],
                _ => return Err("Unknown Valheim world modifier".into()),
            };
            if !values.contains(&value.as_str()) {
                return Err(format!("Unsupported value for {key}"));
            }
        }
        if self.keys.len() > 4
            || self.keys.iter().any(|key| {
                !["nobuildcost", "playerevents", "passivemobs", "nomap"].contains(&key.as_str())
            })
        {
            return Err("Unknown Valheim world rule".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValheimRequest {
    Status,
    SaveSettings {
        expected_revision: String,
        settings: ValheimSettings,
    },
    Package {
        reference: String,
    },
    Install {
        reference: String,
    },
    SetModEnabled {
        package: String,
        enabled: bool,
    },
    RemoveMod {
        package: String,
    },
    CheckUpdates,
    UpdateGame {
        repair: bool,
    },
}

impl ValheimRequest {
    pub fn is_job(&self) -> bool {
        self.changes_files() || matches!(self, Self::CheckUpdates)
    }

    pub fn changes_files(&self) -> bool {
        matches!(
            self,
            Self::Install { .. }
                | Self::SetModEnabled { .. }
                | Self::RemoveMod { .. }
                | Self::UpdateGame { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_real_valheim_settings() {
        let mut settings = ValheimSettings::default();
        assert!(settings.validate("Vikings", true).is_ok());
        assert!(settings.validate("Vikings", false).is_err());
        settings.password = "secret123".into();
        assert!(settings.validate("Vikings", false).is_ok());
        settings.world = "../world".into();
        assert!(settings.validate("Vikings", false).is_err());
        settings.world = "Our world".into();
        settings.modifiers.insert("resources".into(), "most".into());
        assert!(settings.validate("Vikings", false).is_ok());
        settings
            .modifiers
            .insert("resources".into(), "unlimited".into());
        assert!(settings.validate("Vikings", false).is_err());
    }
}
