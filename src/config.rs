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
    pub runtime: Runtime,
    pub environment: Toggle,
    pub session: Toggle,
    pub prompt: Prompt,
    pub colors: Colors,
    pub segments: Segments,
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
    pub cache_ms: u64,
}
impl Default for Git {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 300,
            cache_ms: 1000,
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

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Runtime {
    pub enabled: bool,
    pub timeout_ms: u64,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 250,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Toggle {
    pub enabled: bool,
}
impl Default for Toggle {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    #[default]
    Auto,
    OneLine,
    TwoLine,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Prompt {
    pub success_symbol: String,
    pub error_symbol: String,
    pub layout: Layout,
}
impl Default for Prompt {
    fn default() -> Self {
        Self {
            success_symbol: ">".into(),
            error_symbol: ">".into(),
            layout: Layout::Auto,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub directory: String,
    pub git: String,
    pub duration: String,
    pub status: String,
    pub runtime: String,
    pub environment: String,
    pub session: String,
    pub success: String,
    pub error: String,
}
impl Default for Colors {
    fn default() -> Self {
        Self {
            directory: "cyan".into(),
            git: "magenta".into(),
            duration: "yellow".into(),
            status: "red".into(),
            runtime: "blue".into(),
            environment: "green".into(),
            session: "yellow".into(),
            success: "green".into(),
            error: "red".into(),
        }
    }
}

pub fn color_code(name: &str) -> Option<u8> {
    Some(match name {
        "black" => 30,
        "red" => 31,
        "green" => 32,
        "yellow" => 33,
        "blue" => 34,
        "magenta" => 35,
        "cyan" => 36,
        "white" => 37,
        "bright-black" => 90,
        "bright-red" => 91,
        "bright-green" => 92,
        "bright-yellow" => 93,
        "bright-blue" => 94,
        "bright-magenta" => 95,
        "bright-cyan" => 96,
        "bright-white" => 97,
        _ => return None,
    })
}

pub const SEGMENT_NAMES: [&str; 7] = [
    "session",
    "directory",
    "git",
    "environment",
    "runtime",
    "duration",
    "status",
];

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Segments {
    pub order: Vec<String>,
}
impl Default for Segments {
    fn default() -> Self {
        Self {
            order: SEGMENT_NAMES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
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
    parse(&text).map_err(|e| format!("Invalid config {}: {e}", path.display()))
}

pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    if !(8..=200).contains(&config.directory.max_width) {
        return Err("directory.max_width must be between 8 and 200".into());
    }
    if !(10..=2000).contains(&config.git.timeout_ms) {
        return Err("git.timeout_ms must be between 10 and 2000".into());
    }
    if config.git.cache_ms > 5000 {
        return Err("git.cache_ms must be between 0 and 5000".into());
    }
    if !(10..=2000).contains(&config.runtime.timeout_ms) {
        return Err("runtime.timeout_ms must be between 10 and 2000".into());
    }
    for symbol in [&config.prompt.success_symbol, &config.prompt.error_symbol] {
        if symbol.chars().any(|c| c.is_control())
            || crate::render::sanitize(symbol) != *symbol
            || !(1..=8).contains(&unicode_width::UnicodeWidthStr::width(symbol.as_str()))
        {
            return Err(
                "Prompt symbols must contain 1–8 printable columns, without control characters"
                    .into(),
            );
        }
    }
    for name in [
        &config.colors.directory,
        &config.colors.git,
        &config.colors.duration,
        &config.colors.status,
        &config.colors.runtime,
        &config.colors.environment,
        &config.colors.session,
        &config.colors.success,
        &config.colors.error,
    ] {
        if color_code(name).is_none() {
            return Err(format!("Unknown color: {name}"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for name in &config.segments.order {
        if !SEGMENT_NAMES.contains(&name.as_str()) || !seen.insert(name) {
            return Err("segments.order must contain unique known segment names".into());
        }
    }
    Ok(config)
}

/// Preserve comments, validate the complete document, then replace it atomically.
/// Existing configuration permissions are retained; symlink targets are not edited.
pub fn edit(
    path: &Path,
    change: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Write;
    let metadata = match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("Refusing to edit a symlink or non-regular configuration file".into());
            }
            Some(meta)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let text = if metadata.is_some() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        EXAMPLE.into()
    };
    parse(&text)?;
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| e.to_string())?;
    change(&mut document)?;
    let text = document.to_string();
    parse(&text)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(meta) = metadata {
        temp.as_file()
            .set_permissions(meta.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
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
        assert_eq!(config.git.timeout_ms, 300);
        assert!(config.duration.enabled);
    }

    #[test]
    fn invalid_themes_and_misspelled_fields_are_rejected() {
        assert!(toml::from_str::<Config>("theme = 'unknown'").is_err());
        assert!(toml::from_str::<Config>("colour = true").is_err());
        assert!(toml::from_str::<Config>("[git]\ntimout_ms = 50").is_err());
    }

    #[test]
    fn unsafe_symbols_colors_and_segment_orders_are_rejected() {
        for text in [
            "[prompt]\nsuccess_symbol = ''",
            "[prompt]\nerror_symbol = '\u{202e}'",
            "[colors]\ndirectory = 'not-a-color'",
            "[segments]\norder = ['git', 'git']",
            "[segments]\norder = ['execute-command']",
            "[git]\ncache_ms = 9000",
            "[runtime]\ntimeout_ms = 1",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
