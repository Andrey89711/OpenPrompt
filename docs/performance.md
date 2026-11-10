# Local measurements — OpenPrompt 0.2.0

Measured on Windows x64, Intel Core i5-13420H, release build (`opt-level=s`, LTO),
using `scripts/benchmark.ps1` with 20 samples per repeated scenario. Values include
the full CLI process startup, configuration loading, optional probes, and output
capture from PowerShell. They do **not** measure the whole interactive prompt.

Default budgets: Git 300 ms, shared uncached runtimes 250 ms; Git TTL 1000 ms,
successful runtime versions TTL 60 s. No custom user configuration was written.

| Scenario | OpenPrompt checkout median / p95 | Synthetic repo median / p95 |
| --- | --- | --- |
| Baseline, optional context disabled | 42.83 / 63.91 ms | 29.91 / 42.88 ms |
| Fresh Git, ordinary runtime cache policy | 188.66 / 216.25 ms | 223.23 / 255.98 ms |
| Normal warm cache policy | 60.19 / 67.80 ms | 31.26 / 34.41 ms |

First observed invocations: 307.76 ms for the checkout and 295.64 ms for the
synthetic repository. These are not flushed-OS-cache measurements. The checkout
had source changes and a contextual Rust runtime; the fixture had no runtime
project markers. Earlier runs on the same machine varied significantly under
concurrent build load: warm checkout medians ranged from roughly 33–60 ms.
These results are observations, not latency guarantees or cross-platform claims.

The synthetic repository contained 5000 committed small files, 100 modified
tracked files, and 100 untracked files. A fresh render correctly reported
`git:main* modified:100 new:100`. The temporary repository was removed afterward.

## Limitations and reproduction

- Warm Git results can be stale for up to the configured TTL; `--fresh` bypasses
  only Git caching. Slow or missing runtimes can still consume the probe budget.
- Results depend on antivirus scanning, disk cache, tool shims, repository state,
  PowerShell startup/capture, filesystem, and concurrent workloads.
- No real network share was available for testing. Network-drive filesystem
  operations are not subject to a hard end-to-end deadline.
- Git output exceeding 64 KiB omits the segment; partial counts are not shown.
- The release workflow builds/tests other platforms; these timing measurements
  only describe this local Windows host.

```powershell
cargo build --release --locked
.\scripts\benchmark.ps1 -Executable .\target\release\openprompt.exe -Path . -Iterations 20
```
