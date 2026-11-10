# Changelog

## 0.2.1

- Fix a Windows-only mutable binding that caused Linux/macOS Clippy checks to
  reject an otherwise successful build with `-D warnings`.

## 0.2.0

- Deterministic theme previews, including JSON/ANSI output for integrations.
- Theme selection and style configuration with comment-preserving atomic writes.
- Configurable colors, input symbols, layout, and segment order.
- Git staged/modified/untracked/conflict counts and short-lived status caching.
- Contextual Node/Python/Rust versions, active Python environments, SSH and Windows
  administrator session indicators; successful runtime probe caching.
- Cache clearing, fresh Git rendering, and reproducible local benchmarks.
- OpenTerminal settings integration: read-only preview, explicit theme save,
  font selection, and confirmation before initializing a PowerShell session.
- Version-tag release workflow for Windows x64, Linux x64, and macOS ARM64/x64,
  with target-specific dependency notices and SHA256 checksums.
- Verified local Windows ZIP installer. No automatic profile/PATH changes,
  font installation, or package-manager registrations.

## 0.1.0

- Initial Rust CLI, three themes, TOML configuration, PowerShell 5.1/7 integration,
  directory/Git/status/duration segments, and MIT licensing.
