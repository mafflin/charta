use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "charta";
const SETTINGS_FILE: &str = "settings.toml";
const NOTES_SUBDIR: &str = "notes";
/// Fallbacks when the XDG variables are unset, relative to `$HOME`.
const DEFAULT_CONFIG_HOME: &str = ".config";
const DEFAULT_DATA_HOME: &str = ".local/share";
const HOME_PREFIX: &str = "~";
const ENV_HOME: &str = "HOME";
const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
const ENV_XDG_DATA_HOME: &str = "XDG_DATA_HOME";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Git remote URL of the notes repo. `None` means local-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// When to sync with the remote.
    #[serde(default)]
    pub autosync: Autosync,
    /// Directory holding the notes repo. May start with `~`.
    #[serde(default = "default_notes_dir")]
    pub notes_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repo: None,
            autosync: Autosync::default(),
            notes_dir: default_notes_dir(),
        }
    }
}

/// When to sync with the remote. Use [`Config::sync_mode`] to read it: it's
/// ignored for local-only notes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Autosync {
    /// Pull before opening the editor, push after.
    Full,
    /// Pull and push after a change only, so the editor opens right away.
    #[default]
    Save,
    /// Never; sync with `charta sync`.
    Off,
}

impl Autosync {
    pub const ALL: [Self; 3] = [Self::Full, Self::Save, Self::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Save => "save",
            Self::Off => "off",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }

    pub fn names() -> String {
        Self::ALL.map(Self::as_str).join(", ")
    }
}

impl Config {
    /// Loads the config, or returns `None` if charta hasn't been set up yet.
    pub fn load() -> Result<Option<Self>> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config =
            toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Some(config))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, toml::to_string(self)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    /// How the notes actually sync: local-only notes never do, whatever
    /// `autosync` says.
    pub fn sync_mode(&self) -> Autosync {
        if self.is_local_only() {
            Autosync::Off
        } else {
            self.autosync
        }
    }

    pub fn is_local_only(&self) -> bool {
        self.repo.is_none()
    }

    pub fn notes_path(&self) -> Result<PathBuf> {
        expand_tilde(&self.notes_dir)
    }
}

pub fn config_path() -> Result<PathBuf> {
    let base = match env::var_os(ENV_XDG_CONFIG_HOME).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(DEFAULT_CONFIG_HOME),
    };
    Ok(base.join(APP_NAME).join(SETTINGS_FILE))
}

/// Kept unexpanded (with `~`) when it's the `$HOME` fallback, so the
/// settings file stays portable.
fn default_notes_dir() -> String {
    let base = match env::var_os(ENV_XDG_DATA_HOME).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(HOME_PREFIX).join(DEFAULT_DATA_HOME),
    };
    base.join(APP_NAME).join(NOTES_SUBDIR).display().to_string()
}

fn home() -> Result<PathBuf> {
    env::var_os(ENV_HOME)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .with_context(|| format!("${ENV_HOME} is not set"))
}

pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    let Some(rest) = path.strip_prefix(HOME_PREFIX) else {
        return Ok(PathBuf::from(path));
    };
    if rest.is_empty() {
        return home();
    }
    match rest.strip_prefix(std::path::MAIN_SEPARATOR) {
        Some(rest) => Ok(home()?.join(rest)),
        None => Ok(PathBuf::from(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.repo, None);
        assert_eq!(config.autosync, Autosync::Save);
        assert!(Path::new(&config.notes_dir).ends_with(Path::new(APP_NAME).join(NOTES_SUBDIR)));
    }

    #[test]
    fn roundtrip() {
        let config = Config {
            repo: Some("git@example.com:me/notes.git".into()),
            autosync: Autosync::Save,
            notes_dir: "~/notes".into(),
        };
        let text = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn autosync_modes_roundtrip_as_strings() {
        for mode in Autosync::ALL {
            let config = Config {
                autosync: mode,
                ..Config::default()
            };
            let text = toml::to_string(&config).unwrap();
            let line = format!("autosync = \"{}\"", mode.as_str());
            assert!(text.contains(&line), "{text}");
            assert_eq!(toml::from_str::<Config>(&text).unwrap().autosync, mode);
        }
    }

    #[test]
    fn autosync_rejects_unknown_modes() {
        let err = toml::from_str::<Config>("autosync = \"sometimes\"").unwrap_err();
        assert!(err.to_string().contains("full"), "{err}");
        assert!(toml::from_str::<Config>("autosync = true").is_err());
        assert!(toml::from_str::<Config>("autosync = \"Full\"").is_err());
    }

    #[test]
    fn local_only_never_syncs() {
        let local = Config {
            autosync: Autosync::Full,
            ..Config::default()
        };
        assert_eq!(local.sync_mode(), Autosync::Off);
        let remote = Config {
            repo: Some("git@example.com:me/notes.git".into()),
            ..local
        };
        assert_eq!(remote.sync_mode(), Autosync::Full);
    }

    #[test]
    fn local_only_omits_repo() {
        let text = toml::to_string(&Config::default()).unwrap();
        assert!(!text.contains("repo"));
    }

    #[test]
    fn non_tilde_paths_are_untouched() {
        assert_eq!(expand_tilde("/tmp/x").unwrap(), PathBuf::from("/tmp/x"));
        assert_eq!(expand_tilde("~x").unwrap(), PathBuf::from("~x"));
    }
}
