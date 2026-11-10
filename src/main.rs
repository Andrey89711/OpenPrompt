mod cache;
mod config;
mod git;
mod render;
mod segments;

use base64::Engine;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "A fast, configurable prompt for PowerShell")]
struct Cli {
    /// Use an explicit TOML configuration file.
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render the current context without a trailing newline.
    Render(RenderArgs),
    /// Preview all themes using safe sample data, without executing Git or runtimes.
    Preview {
        #[arg(long, value_enum)]
        theme: Option<ThemeArg>,
        /// Machine-readable {theme, ansi, text}; requires --theme.
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_color: bool,
    },
    /// Print shell integration. Never edits a profile.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List the built-in themes.
    Themes,
    /// Persist a theme selection, preserving comments and unrelated settings.
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },
    /// Customize colors, symbols, layout, and segment order.
    Style {
        #[command(subcommand)]
        command: StyleCommand,
    },
    /// Manage the short-lived Git cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[derive(Args)]
struct RenderArgs {
    #[arg(long, value_enum, default_value = "powershell")]
    shell: Shell,
    #[arg(long)]
    path: Option<PathBuf>,
    /// Previous exit status; 0 means success.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    status: i64,
    /// Previous command duration in milliseconds.
    #[arg(long, default_value_t = 0)]
    duration: u64,
    #[arg(long, default_value_t = 80)]
    width: u16,
    #[arg(long)]
    no_color: bool,
    #[arg(long, value_enum)]
    theme: Option<ThemeArg>,
    /// Bypass the Git cache for this render.
    #[arg(long)]
    fresh: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Powershell,
}

#[derive(Clone, Copy, ValueEnum)]
enum ThemeArg {
    Minimal,
    Classic,
    Blocks,
}
impl ThemeArg {
    fn name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Classic => "classic",
            Self::Blocks => "blocks",
        }
    }
    fn value(self) -> config::Theme {
        match self {
            Self::Minimal => config::Theme::Minimal,
            Self::Classic => config::Theme::Classic,
            Self::Blocks => config::Theme::Blocks,
        }
    }
}

#[derive(Subcommand)]
enum ConfigCommand {
    Path,
    Init,
    Check,
}
#[derive(Subcommand)]
enum ThemeCommand {
    Set {
        #[arg(value_enum)]
        theme: ThemeArg,
    },
}
#[derive(Subcommand)]
enum CacheCommand {
    Clear,
}
#[derive(Subcommand)]
enum StyleCommand {
    Set(StyleArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum LayoutArg {
    Auto,
    OneLine,
    TwoLine,
}
impl LayoutArg {
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::OneLine => "one-line",
            Self::TwoLine => "two-line",
        }
    }
}

#[derive(Args)]
struct StyleArgs {
    #[arg(long)]
    success_symbol: Option<String>,
    #[arg(long)]
    error_symbol: Option<String>,
    #[arg(long, value_enum)]
    layout: Option<LayoutArg>,
    /// Named color assignment, repeatable: --color directory=cyan.
    #[arg(long, value_name = "SEGMENT=COLOR")]
    color: Vec<String>,
    /// Ordered comma-separated segment names. Omitted names are hidden.
    #[arg(long, value_delimiter = ',')]
    order: Option<Vec<String>>,
    #[arg(long, value_parser = clap::value_parser!(bool))]
    icons: Option<bool>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(output) => match io::stdout().lock().write_all(output.as_bytes()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("openprompt: {e}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("openprompt: {error}");
            ExitCode::FAILURE
        }
    }
}

fn powershell_literal(value: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    format!("([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')))")
}

fn config_path(explicit: Option<&Path>) -> Result<PathBuf, String> {
    explicit
        .map(Path::to_path_buf)
        .map_or_else(config::default_path, Ok)
}

fn use_color(no_color: bool) -> bool {
    !no_color
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").unwrap_or_default() != "dumb"
}

fn run(cli: Cli) -> Result<String, String> {
    match cli.command {
        Commands::Init { shell: Shell::Powershell } => {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            // ASCII-only script survives Windows PowerShell's OEM stdout decoder.
            let literal = powershell_literal(&executable.to_string_lossy());
            let config_literal = match cli.config {
                Some(path) => powershell_literal(&std::path::absolute(path).map_err(|e| e.to_string())?.to_string_lossy()),
                None => "$null".into(),
            };
            Ok(include_str!("../shells/powershell.ps1")
                .replace("__OPENPROMPT_EXECUTABLE__", &literal)
                .replace("__OPENPROMPT_CONFIG__", &config_literal))
        }
        Commands::Themes => Ok("minimal  One line, compact context\nclassic  Two lines, clean and readable (default)\nblocks   Colored blocks, two lines\n".into()),
        Commands::Config { command } => {
            let path = config_path(cli.config.as_deref())?;
            match command {
                ConfigCommand::Path => Ok(format!("{}\n", path.display())),
                ConfigCommand::Init => { config::init(&path)?; Ok(format!("Created {}\n", path.display())) }
                ConfigCommand::Check => { config::load(Some(&path))?; Ok(format!("Configuration is valid: {}\n", path.display())) }
            }
        }
        Commands::Theme { command: ThemeCommand::Set { theme } } => {
            let path = config_path(cli.config.as_deref())?;
            config::edit(&path, |document| { document["theme"] = toml_edit::value(theme.name()); Ok(()) })?;
            Ok(format!("Theme set to {} in {}\n", theme.name(), path.display()))
        }
        Commands::Style { command: StyleCommand::Set(style) } => {
            if style.success_symbol.is_none() && style.error_symbol.is_none() && style.layout.is_none() && style.color.is_empty() && style.order.is_none() && style.icons.is_none() {
                return Err("Specify at least one style setting; see openprompt style set --help".into());
            }
            let path = config_path(cli.config.as_deref())?;
            config::edit(&path, |document| {
                if let Some(value) = style.success_symbol { document["prompt"]["success_symbol"] = toml_edit::value(value); }
                if let Some(value) = style.error_symbol { document["prompt"]["error_symbol"] = toml_edit::value(value); }
                if let Some(value) = style.layout { document["prompt"]["layout"] = toml_edit::value(value.name()); }
                if let Some(value) = style.icons { document["icons"] = toml_edit::value(value); }
                for assignment in style.color {
                    let (key, value) = assignment.split_once('=').ok_or("Use --color SEGMENT=COLOR")?;
                    if !["directory", "git", "duration", "status", "runtime", "environment", "session", "success", "error"].contains(&key) {
                        return Err(format!("Unknown color target: {key}"));
                    }
                    if config::color_code(value).is_none() { return Err(format!("Unknown named color: {value}")); }
                    document["colors"][key] = toml_edit::value(value);
                }
                if let Some(order) = style.order {
                    let mut array = toml_edit::Array::new();
                    for name in order { array.push(name); }
                    document["segments"]["order"] = toml_edit::value(array);
                }
                Ok(())
            })?;
            Ok(format!("Style saved in {}\n", path.display()))
        }
        Commands::Cache { command: CacheCommand::Clear } => Ok(format!("Removed {} Git and runtime cache entries\n", cache::clear()? + segments::clear_cache()?)),
        Commands::Preview { theme, json, no_color } => {
            if json && theme.is_none() { return Err("Use --theme <NAME> with --json".into()); }
            let mut config = config::load(cli.config.as_deref())?;
            let themes = theme.map_or_else(|| vec![ThemeArg::Minimal, ThemeArg::Classic, ThemeArg::Blocks], |theme| vec![theme]);
            let mut output = String::new();
            for theme in themes {
                config.theme = theme.value();
                let git = git::GitStatus { branch: "main".into(), dirty: true, ahead: 2, staged: 1, ..Default::default() };
                let mut extra = Vec::new();
                if config.environment.enabled { extra.push("venv:demo".into()); }
                if config.runtime.enabled { extra.push("node:v22.0.0".into()); }
                let mut context = render::Context { path: Path::new("Projects/OpenPrompt"), home: None, status: 1, duration_ms: 1250, width: 160, color: true, git: Some(&git), extra: &extra };
                let ansi = render::render(&config, &context);
                context.color = false;
                let text = render::render(&config, &context);
                if json { return serde_json::to_string(&serde_json::json!({ "theme": theme.name(), "ansi": ansi, "text": text })).map_err(|e| e.to_string()); }
                output.push_str(&format!("{}\n{}\n\n", theme.name(), if use_color(no_color) { ansi } else { text }));
            }
            Ok(output)
        }
        Commands::Render(args) => {
            let mut config = config::load(cli.config.as_deref())?;
            if let Some(theme) = args.theme { config.theme = theme.value(); }
            let path = match args.path { Some(path) => path, None => std::env::current_dir().map_err(|e| e.to_string())? };
            let git = if config.git.enabled { git::detect_cached(&path, Duration::from_millis(config.git.timeout_ms), if args.fresh { 0 } else { config.git.cache_ms }) } else { None };
            let extra = segments::detect(&path, &segments::Options {
                runtimes: config.runtime.enabled,
                environment: config.environment.enabled,
                session: config.session.enabled,
                timeout: Duration::from_millis(config.runtime.timeout_ms),
            });
            Ok(render::render(&config, &render::Context {
                path: &path, home: dirs::home_dir().as_deref(), status: args.status, duration_ms: args.duration,
                width: usize::from(args.width), color: use_color(args.no_color), git: git.as_ref(), extra: &extra,
            }))
        }
    }
}
