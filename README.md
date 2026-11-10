# OpenPrompt

A small, configurable command prompt for **PowerShell 7 and Windows PowerShell 5.1**, written in Rust.

OpenPrompt styles the text **inside** your terminal. It does not replace PowerShell
or provide a terminal window. Use it with OpenTerminal, Windows Terminal, VS Code,
or another ANSI-capable terminal.

```text
~\Projects\OpenPrompt  git:main*  1.2s
>
```

**v0.1:** PowerShell integration, three themes, TOML configuration, directory/Git/status/duration segments.
Bash/Zsh integrations, runtime versions, and a theme editor are not implemented yet.

## Install from source

Requirements: stable Rust, Cargo, PowerShell. Git is optional (only the Git segment needs it).
On Windows, Rust's MSVC toolchain also requires Visual Studio C++ Build Tools and the Windows SDK.

```powershell
git clone https://github.com/Andrey89711/OpenPrompt.git
cd OpenPrompt
cargo install --path . --locked
openprompt --version
```

Cargo installs the executable in `~/.cargo/bin`. Ensure that directory is in your `PATH`.
No administrator privileges are required.

## Try it without changing your profile

```powershell
Invoke-Expression (& openprompt init powershell | Out-String)
```

Return to the original prompt in this session:

```powershell
Disable-OpenPrompt
```

Initialization embeds the executable's absolute path, including paths with spaces,
apostrophes, and Unicode characters. It does not download or execute remote scripts.
Review the integration before evaluating it if desired:

```powershell
openprompt init powershell
```

### Enable on every start

Back up your PowerShell profile, then **manually** add this line to `$PROFILE`:

```powershell
Invoke-Expression (& openprompt init powershell | Out-String)
```

OpenPrompt never edits your profile automatically. Remove the line to uninstall
the integration. Do not initialize Oh My Posh or Starship alongside OpenPrompt;
whichever replaces `prompt` last takes over.

## Themes

```powershell
openprompt themes
openprompt render --theme minimal
openprompt render --theme classic
openprompt render --theme blocks
```

| Theme | Appearance |
| --- | --- |
| `minimal` | One compact line |
| `classic` | Context above the input line (default) |
| `blocks` | Colored background blocks above the input line |

No special font is required by default. For optional icons, install and select a
Nerd Font in your terminal, then set `icons = true`. OpenPrompt does not install fonts.

## Configuration

```powershell
openprompt config path
openprompt config init
openprompt config check
```

The default file is `~/.config/openprompt/config.toml`, or
`$XDG_CONFIG_HOME/openprompt/config.toml` when that variable is an absolute path.
Missing default configuration uses built-in defaults. `config init` never overwrites
an existing file. Unknown settings and invalid values produce an error.

```toml
theme = "classic" # minimal | classic | blocks
icons = false

[directory]
enabled = true
max_width = 40

[git]
enabled = true
timeout_ms = 150

[duration]
enabled = true
min_ms = 1000

[status]
enabled = true
```

Preview an alternative configuration with `openprompt --config ./custom.toml render`.
To use it in the shell integration, initialize with that configuration:

```powershell
Invoke-Expression (& openprompt --config ./custom.toml init powershell | Out-String)
```

The file location is converted to an absolute path during initialization.

### Segment behavior

- **Directory:** shortens the home directory to `~` and truncates long paths.
- **Git:** branch (or short detached commit), `*` for changes, `+N` ahead, `-N` behind
  the configured upstream. No network requests. Internal submodule changes are skipped.
- **Status:** `exit:N` for an unsuccessful command; the input symbol also turns red.
  PowerShell cmdlet failures may only provide a success/failure flag, not a native exit code.
- **Duration:** elapsed execution time from PowerShell command history, not idle time.
  Hosts that do not provide history omit it; repeated prompts may reuse the last history entry.

Git output is bounded to 64 KiB and checks have a configurable time budget. Missing
Git, inaccessible repositories, or timeout simply omit the segment. Rendering still
starts a native process each time; performance depends on your machine and repository.

`--no-color`, the presence of `NO_COLOR`, or `TERM=dumb` disables ANSI colors.
Directory and branch names have terminal control and bidirectional control characters removed.
Colors are reset after every segment. Width fitting is Unicode-aware, but font/emoji
display width can still differ between terminal implementations.

## Reliability

The integration preserves `$LASTEXITCODE`, restores any temporary output-encoding
change, and falls back to a plain `PS path> ` if the renderer fails. Non-filesystem
PowerShell providers use the plain fallback. No background daemon or network service.

OpenPrompt currently targets PowerShell. The Rust CLI builds on Linux/macOS too;
PowerShell must still be installed to use the integration there. Native Bash/Zsh
escaping and shell integrations are separate future work.

## Development and tests

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
.\scripts\test-powershell.ps1 -Executable .\target\release\openprompt.exe
```

Rust tests cover configuration, rendering, control-character handling, Unicode,
width fitting, CLI commands, and a temporary Git repository. PowerShell integration
tests run isolated `-NoProfile` child shells; they do not modify your profile.

The release executable is `target/release/openprompt.exe` on Windows.
Unlike a desktop GUI, this is intentionally a console CLI: call it from your shell,
not by double-clicking the executable.

## Roadmap

- Bash and Zsh integration, with correct nonprinting-character markers.
- Additional runtime/environment segments with bounded execution and caching.
- Theme preview and configuration in OpenTerminal.
- More shell-host tests and measured startup benchmarks.

## License

[MIT](LICENSE), copyright (c) 2026 Andrey89711.
Dependency licenses remain applicable to their respective code.
