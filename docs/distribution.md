# Distribution and local installation

OpenPrompt is free, MIT-licensed software. There is no signup, license purchase,
or account required to run it. A GitHub account is not needed to download public
release assets.

## Release archives

An explicit Git tag matching the Cargo package version (for example, `v0.2.0`)
starts `.github/workflows/release.yml`. Do not tag until the version and lockfile
are ready. The workflow checks the version, runs `cargo test --locked` on each
runner, and builds with `cargo build --release --locked --target TARGET`. Windows
also runs `scripts/test-powershell.ps1` against the target release executable.

| Target | Runner | Archive |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | `windows-latest` | `.zip` |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `.tar.gz` |
| `aarch64-apple-darwin` | `macos-15` | `.tar.gz` |
| `x86_64-apple-darwin` | `macos-15-intel` | `.tar.gz` |

Apple Silicon uses the native ARM64 `macos-15` runner; Intel uses
`macos-15-intel`. Both labels are listed in the current
[GitHub runner images documentation](https://github.com/actions/runner-images#available-images).
The old `macos-14` label retired from GitHub Actions on November 2, 2026.
Tests run natively on each runner. Availability of these hosted runner labels is a
prerequisite for release; a missing runner must not be worked around by silently
publishing an incomplete release.

Each archive contains exactly four top-level files: the executable, `LICENSE`,
`README.md`, and `THIRD_PARTY_NOTICES.txt`.
Names follow `openprompt-vVERSION-TARGET.zip` or
`openprompt-vVERSION-TARGET.tar.gz`. Only after every build succeeds does the
publish job download the actual archives, generate `SHA256SUMS`, and publish a
non-draft GitHub Release with generated notes. Only that job has repository
contents write permission. The workflow does not sign or notarize binaries;
checksums detect mismatches, but are not independent proof of publisher identity.

Before packaging, `python scripts/third-party-notices.py` generates the notices
using target-filtered `cargo metadata --locked --format-version 1 --filter-platform TARGET`.
It excludes the root package and includes reachable target dependencies (extra
build/test dependency notices can also be included). Each crate's name, version, declared SPDX expression, and actual
license text files are included. Only regular, in-package `license*`/`copying*`
files or the declared `license_file`, with no extension or `.txt`/`.md`, are read;
no license text is invented. Missing readable text fails the release rather than
silently omitting a dependency. To generate a local check without creating a root
file, use `python scripts/third-party-notices.py --output target/THIRD_PARTY_NOTICES.txt`
after Cargo has created `target/`.

## Windows: verified local ZIP installation

Download the Windows ZIP and `SHA256SUMS` from the same trusted release. Read the
checksum for the exact ZIP filename and pass its 64 hexadecimal characters to
the installer. Do not substitute an example or an unverified checksum.

```powershell
# Copy the real hash for this exact archive from the release's SHA256SUMS.
$expectedSha256 = Read-Host 'Expected SHA256 (64 hex characters)'
.\scripts\install.ps1 `
    -Archive .\openprompt-v0.2.0-x86_64-pc-windows-msvc.zip `
    -Sha256 $expectedSha256
```

The installer accepts only a local ZIP; it never downloads or executes archive
contents. It verifies the hash before extraction, requires exactly the expected
four top-level files, and rejects duplicate entries, path traversal, links, and
unexpected files. It installs `openprompt.exe` and prefixed license/notice sidecars to `$HOME/.local/bin` by
default. Use `-Destination C:\Tools\OpenPrompt` to choose another directory.
Existing executables are not overwritten unless `-Force` is supplied. Temporary
extraction is cleaned up, and neither PATH nor PowerShell profiles are changed.

Invoke the executable by full path, or add the directory to PATH yourself. Shell
integration remains a separate, explicit action described in `README.md`.

For Linux/macOS, verify the matching archive against `SHA256SUMS` first (for
example, `sha256sum` on Linux or `shasum -a 256` on macOS), then unpack and place
the executable in a directory you control. Do not run unverified downloads.

## Reproducible local timing

Build the release executable and benchmark a specific working directory:

```powershell
cargo build --release --locked
.\scripts\benchmark.ps1 -Executable .\target\release\openprompt.exe `
    -Path C:\src\my-project -Iterations 20
```

The script reports full CLI process wall-clock latency in milliseconds, including
startup and configuration loading: one first invocation, then median and
nearest-rank p95 for a temporary baseline with Git/runtime/environment/session
modules disabled, normal configuration with `--fresh` Git-cache bypass, and
normal warm renders. The default is 20 samples per repeated scenario, capped at
200. Normal configuration is left unchanged; only the baseline uses a temporary
configuration. Warm rendering can still refresh an expired cache. First
invocation is not a guarantee of a cold filesystem or OS cache. These are CLI
measurements, not timings of the complete interactive PowerShell prompt.

Record the executable version, OS, machine, directory/repository state, enabled
modules, and sample count with any published measurements. No performance claim
or sample timing here is a substitute for running the benchmark.

## Scoop and WinGet

Scoop and WinGet publication is deferred. No package-manager manifest or listing
is claimed by this repository. Before submitting either package:

1. Publish and test a real versioned release through the workflow.
2. Use the exact immutable versioned asset URL and its verified SHA256 from
   that release. Never invent checksums or use a moving `latest` URL.
3. Validate the manifest with the package manager's current tooling and review
   its installation, upgrade, removal, and PATH behavior on a clean machine.
4. Submit through the relevant bucket or community repository's review process;
   only document package-manager installation after the package is accepted.

The local installer is not a Scoop/WinGet registration and does not bypass their
package review requirements.
