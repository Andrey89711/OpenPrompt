use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const EXAMPLE: &str = include_str!("../themes/default.toml");

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Minimal,
    #[default]
    Classic,
    Blocks,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Theme,
    pub icons: bool,
    pub directory: Directory,
    pub git: Git,
    pub duration: Duration,
    pub status: Status,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Directory {
    pub enabled: bool,
    pub max_width: usize,
}
impl Default for Directory {
    fn default() -> Self {
        Self {
            enabled: true,
            max_width: 40,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Git {
    pub enabled: bool,
    pub timeout_ms: u64,
}
impl Default for Git {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 150,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Duration {
    pub enabled: bool,
    pub min_ms: u64,
}
impl Default for Duration {
    fn default() -> Self {
        Self {
            enabled: true,
            min_ms: 1000,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Status {
    pub enabled: bool,
}
impl Default for Status {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub fn default_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(path.join("openprompt/config.toml"));
    }
    dirs::home_dir()
        .map(|p| p.join(".config/openprompt/config.toml"))
        .ok_or_else(|| "Cannot locate home directory; use --config <FILE>".into())
}

pub fn load(explicit: Option<&Path>) -> Result<Config, String> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => default_path()?,
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
            return Ok(Config::default())
        }
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    let config: Config =
        toml::from_str(&text).map_err(|e| format!("Invalid config {}: {e}", path.display()))?;
    if !(8..=200).contains(&config.directory.max_width) {
        return Err("directory.max_width must be between 8 and 200".into());
    }
    if !(10..=2000).contains(&config.git.timeout_ms) {
        return Err("git.timeout_ms must be between 10 and 2000".into());
    }
    Ok(config)
}

pub fn init(path: &Path) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create config directory: {e}"))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            format!(
                "Cannot create {} (existing files are never overwritten): {e}",
                path.display()
            )
        })?;
    file.write_all(EXAMPLE.as_bytes())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_example_matches_defaults() {
        let config: Config = toml::from_str(EXAMPLE).unwrap();
        assert_eq!(config.theme, Theme::Classic);
        assert!(config.git.enabled);
        assert_eq!(config.directory.max_width, 40);
        assert!(!config.icons);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let config: Config = toml::from_str("theme = 'minimal'\n[git]\nenabled = false").unwrap();
        assert_eq!(config.theme, Theme::Minimal);
        assert!(!config.git.enabled);
        assert_eq!(config.git.timeout_ms, 150);
        assert!(config.duration.enabled);
    }

    #[test]
    fn invalid_themes_and_misspelled_fields_are_rejected() {
        assert!(toml::from_str::<Config>("theme = 'unknown'").is_err());
        assert!(toml::from_str::<Config>("colour = true").is_err());
        assert!(toml::from_str::<Config>("[git]\ntimout_ms = 50").is_err());
    }
}
