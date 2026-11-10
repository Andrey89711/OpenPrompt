mod config;
mod git;
mod render;

use base64::Engine;
use clap::{Parser, Subcommand, ValueEnum};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "A fast, configurable prompt for PowerShell", long_about = None)]
struct Cli {
    /// Use an explicit TOML configuration file.
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render a prompt without a trailing newline.
    Render {
        #[arg(long, value_enum, default_value = "powershell")]
        shell: Shell,
        /// Working directory. Defaults to the current directory.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Previous command exit status (0 means success).
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        status: i64,
        /// Previous command duration in milliseconds.
        #[arg(long, default_value_t = 0)]
        duration: u64,
        /// Available terminal columns.
        #[arg(long, default_value_t = 80)]
        width: u16,
        /// Do not emit ANSI color sequences. Also respects NO_COLOR and TERM=dumb.
        #[arg(long)]
        no_color: bool,
        /// Override the configured theme for this render.
        #[arg(long, value_enum)]
        theme: Option<ThemeArg>,
    },
    /// Print shell integration. This does not edit your shell profile.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Manage the configuration file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List the built-in themes.
    Themes,
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

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the configuration file location.
    Path,
    /// Create a documented default configuration; never overwrites existing files.
    Init,
    /// Validate the configuration, including an explicitly supplied file.
    Check,
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

fn run(cli: Cli) -> Result<String, String> {
    match cli.command {
        Commands::Init { shell: Shell::Powershell } => {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            // Keep init output ASCII even under Windows PowerShell's OEM decoder.
            // Encoding a path also prevents it from injecting PowerShell syntax.
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
            let path = match &cli.config { Some(path) => path.clone(), None => config::default_path()? };
            match command {
                ConfigCommand::Path => Ok(format!("{}\n", path.display())),
                ConfigCommand::Init => { config::init(&path)?; Ok(format!("Created {}\n", path.display())) }
                ConfigCommand::Check => { config::load(Some(&path))?; Ok(format!("Configuration is valid: {}\n", path.display())) }
            }
        }
        Commands::Render { shell: _, path, status, duration, width, no_color, theme } => {
            let mut config = config::load(cli.config.as_deref())?;
            if let Some(theme) = theme {
                config.theme = match theme { ThemeArg::Minimal => config::Theme::Minimal, ThemeArg::Classic => config::Theme::Classic, ThemeArg::Blocks => config::Theme::Blocks };
            }
            let path = path.unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
            let git = if config.git.enabled { git::detect(&path, Duration::from_millis(config.git.timeout_ms)) } else { None };
            let color = !no_color && std::env::var_os("NO_COLOR").is_none() && std::env::var("TERM").unwrap_or_default() != "dumb";
            Ok(render::render(&config, &render::Context {
                path: &path, home: dirs::home_dir().as_deref(), status, duration_ms: duration,
                width: usize::from(width), color, git: git.as_ref(),
            }))
        }
    }
}
