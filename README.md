# LockLens

**See what actually changed between two lockfiles.**

[![CI](https://github.com/hix-pro/locklens/actions/workflows/ci.yml/badge.svg)](https://github.com/hix-pro/locklens/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/hix-pro/locklens)](https://github.com/hix-pro/locklens/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

A lockfile diff is usually a wall of changed lines. `locklens` reduces it to the
package-level answer: what was added, removed, upgraded, downgraded, or resolved
from a different checksum or source.

One small binary. Reads files only. No registry calls, account, or telemetry.

## Why

Reviewing a lockfile by hand hides the changes that matter. A patch upgrade and
a major upgrade scroll past at the same speed; a package that resolves to the
same version with a different checksum looks unchanged in a line diff.

LockLens groups the diff by package name and classifies each change. It also
compares the metadata that lockfile diffs normally bury:

- **Checksum changed** for the same version is a supply-chain signal. The
  version did not move, but the artifact did.
- **Resolved source changed** for the same version means the package now comes
  from a different registry, host, or git reference.
- **Major and 0.x minor upgrades** are called out as breaking ranges instead of
  being mixed in with patch bumps.

The output is deterministic, the JSON schema is stable, and the exit code is
yours to choose.

## See it

```console
$ locklens diff examples/before/package-lock.json examples/after/package-lock.json
LockLens · 0.1.0
Old examples/before/package-lock.json  npm · 5 packages
New examples/after/package-lock.json  npm · 5 packages

critical (1)
  integrity    left-pad
    checksum changed for version 1.3.0
    old: sha512-left-pad-old
    new: sha512-left-pad-new

warning (2)
  major        vite
    5.4.0 → 6.0.0 (major upgrade)
  downgrade    typescript (dev)
    5.6.0 → 5.5.0 (downgrade)

info (3)
  added        kleur
    4.1.5 (new package)
  removed      chalk
    4.0.0 (removed from the lockfile)
  patch        lodash
    4.17.20 → 4.17.21 (patch upgrade)

Summary: 6 packages changed · 6 findings (1 critical, 2 warnings, 3 info)
Result: report only (use --fail-on critical|error|warning|any to gate CI)
```

The example pair lives in [`examples/`](examples) and is compared by the
repository's own Dogfood workflow on every push.

## Supported lockfiles

| Ecosystem | Files | Versions |
| --- | --- | --- |
| npm | `package-lock.json`, `npm-shrinkwrap.json` | `lockfileVersion` 1, 2, 3 |
| pnpm | `pnpm-lock.yaml` | v5 (`/name/version`), v6 (`/name@version`), v9 (`name@version` with peer sets) |
| Yarn classic | `yarn.lock` | v1 block format |
| Yarn Berry | `yarn.lock` | `__metadata` lockfiles |
| Cargo | `Cargo.lock` | v3 and v4 |
| uv | `uv.lock` | v1 |
| Poetry | `poetry.lock` | `lock-version` 1.x and 2.x |

The format is detected from the filename first, then from the content, so a
lockfile copied to a temporary path still works. Both sides of a comparison may
use different formats: versions are compared normally, while checksum and source
checks are skipped because the formats store that metadata differently.

## Install

From source:

```sh
cargo install --git https://github.com/hix-pro/locklens --locked
```

After the first release, use a prebuilt binary:

```sh
curl -LO https://github.com/hix-pro/locklens/releases/latest/download/locklens-x86_64-unknown-linux-gnu.tar.gz
tar xzf locklens-x86_64-unknown-linux-gnu.tar.gz
install locklens-x86_64-unknown-linux-gnu/locklens /usr/local/bin/
```

For local development:

```sh
git clone https://github.com/hix-pro/locklens
cd locklens
cargo build --release
./target/release/locklens --help
```

Requires Rust 1.85 or newer.

## Quick start

Compare the lockfile on the base branch with the one in your working tree:

```sh
git show origin/main:package-lock.json > /tmp/package-lock.json.base
locklens diff /tmp/package-lock.json.base package-lock.json
```

See a paste-ready pull request comment:

```sh
locklens diff old.lock new.lock --markdown
```

Gate CI on review-worthy findings:

```sh
locklens diff old.lock new.lock --fail-on warning
```

## Commands

```text
locklens diff <old> <new> [--json | --markdown | --sarif] [--fail-on LEVEL] [--ignore PACKAGE]...
```

| Option | Description |
| --- | --- |
| `--json` | Write the stable JSON report to stdout. Cannot be combined with `--markdown` or `--sarif`. |
| `--markdown` | Write a Markdown report suitable for a pull request or release note. |
| `--sarif` | Write a SARIF 2.1.0 report for GitHub code scanning. |
| `--fail-on <level>` | Exit `2` when findings at or above `critical`, `error`, `warning`, or `any` are present. |
| `--ignore <package>`, `-i` | Ignore an exact package name. Repeat for several packages. |

The default mode is a report: changes are printed, but the exit code stays `0`
unless `--fail-on` is supplied.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Comparison succeeded. Includes report-only runs with findings, and runs below the `--fail-on` threshold. |
| `1` | Usage, I/O, or parse error. |
| `2` | Findings matched the `--fail-on` threshold. |

Thresholds are inclusive: `--fail-on warning` also fails on errors and critical
findings; `--fail-on critical` only fails when a checksum changed. `--fail-on
any` includes additions, removals, and patch upgrades.

## What counts as a finding

| Kind | Default severity | Meaning |
| --- | --- | --- |
| `integrity` | critical | Same package version, different checksum |
| `source` | error | Same package version, different resolved source |
| `downgrade` | warning | The version moved backwards |
| `major` | warning | A major upgrade, or a 0.x minor upgrade (a breaking range) |
| `changed` | warning | The resolved version set changed without one clear upgrade, or a version is not valid semver |
| `added` | info | A package appeared in the lockfile |
| `removed` | info | A package disappeared from the lockfile |
| `minor` | info | A minor upgrade |
| `patch` | info | A patch upgrade |

A `changed` finding that only moves a prerelease or build metadata is reported as
`info` instead of `warning`.

Lockfiles can contain several resolved versions of the same package. LockLens
compares the version **set** per package and reports the whole set when it
changes, rather than pretending one version replaced the other.

## How it works

1. Parse each lockfile into `(name, version)` entries with resolved sources,
   checksums, and dev/optional flags.
2. Deduplicate entries that resolve the same package version through different
   paths.
3. Group both sides by package name and compare the version sets.
4. When the same format is used on both sides, compare checksums and sources for
   versions that exist on both sides.
5. Sort by severity, finding kind, and package name for deterministic output.

No dependency graph is built and no registry is contacted. The answer comes from
the two files you gave it.

## CI

Fetch the base lockfile and compare:

```yaml
name: Lockfile review

on: pull_request

permissions:
  contents: read

jobs:
  locklens:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install --git https://github.com/hix-pro/locklens --locked
      - run: git show origin/${{ github.base_ref }}:package-lock.json > /tmp/package-lock.json.base
      - run: locklens diff /tmp/package-lock.json.base package-lock.json --fail-on warning
```

Adjust the path for pnpm, Yarn, Cargo, uv, or Poetry. If the lockfile is in a
workspace subdirectory, use that path on both sides. Start with `--fail-on
error` or `critical` if major upgrades are too noisy for the first rollout.

This repository's own workflow is in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## GitHub Action

The composite action downloads the prebuilt binary for a version tag, runs the
comparison, uploads SARIF by default, and preserves the comparison exit code:

```yaml
permissions:
  contents: read
  security-events: write

steps:
  - uses: actions/checkout@v4
    with:
      fetch-depth: 0
  - run: git show origin/${{ github.base_ref }}:package-lock.json > /tmp/package-lock.json.base
  - uses: hix-pro/locklens@v0.1.0
    with:
      old: /tmp/package-lock.json.base
      new: package-lock.json
      fail-on: warning
```

Inputs: `old` and `new` (required), `fail-on` (`critical`, `error`, `warning`,
`any`, or empty for report-only), `args` for extra CLI flags, `sarif` (default
`true`), and `category` (default `locklens`). With `sarif: true`, the action
writes and uploads a report and then re-exits with the comparison code. The
caller needs `security-events: write`; the upload never changes the result.

Do not pass `--json` or `--markdown` in `args` when SARIF is enabled; the output
formats are mutually exclusive.

## SARIF output

`--sarif` emits [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
for GitHub code scanning and other SARIF consumers:

```sh
locklens diff old.lock new.lock --sarif > locklens.sarif
```

| Rule | Level | Meaning |
| --- | --- | --- |
| `locklens/integrity-changed` | `error` | Same version, different checksum |
| `locklens/source-changed` | `error` | Same version, different source |
| `locklens/major-upgrade` | `warning` | Breaking-range upgrade |
| `locklens/downgrade` | `warning` | Version moved backwards |
| `locklens/version-changed` | `warning` | Version set changed without one clear upgrade |
| `locklens/minor-upgrade` | `note` | Minor upgrade |
| `locklens/patch-upgrade` | `note` | Patch upgrade |
| `locklens/added` | `note` | Package added |
| `locklens/removed` | `note` | Package removed |

Results are file-level annotations on the new lockfile. Removed packages point
at the old lockfile, since that is where the package still exists. The logical
location is the package name and, for version changes, `name@before -> after`.

```yaml
permissions:
  contents: read
  security-events: write

steps:
  - uses: actions/checkout@v4
    with:
      fetch-depth: 0
  - run: git show origin/${{ github.base_ref }}:package-lock.json > /tmp/package-lock.json.base
  - run: cargo install --git https://github.com/hix-pro/locklens --locked
  - run: locklens diff /tmp/package-lock.json.base package-lock.json --sarif --fail-on warning > locklens.sarif
  - uses: github/codeql-action/upload-sarif@v4
    if: always()
    with:
      sarif_file: locklens.sarif
      category: locklens
```

The composite action above does this automatically when `sarif: true`.

## JSON output

`--json` emits a stable, versioned document:

```json
{
  "schema_version": 1,
  "tool": {
    "name": "locklens",
    "version": "0.1.0"
  },
  "old": {
    "path": "examples/before/package-lock.json",
    "format": "npm",
    "lockfile_version": "3",
    "packages": 5
  },
  "new": {
    "path": "examples/after/package-lock.json",
    "format": "npm",
    "lockfile_version": "3",
    "packages": 5
  },
  "ignored": [],
  "summary": {
    "total": 6,
    "packages_changed": 6,
    "unchanged": 0,
    "critical": 1,
    "error": 0,
    "warning": 2,
    "info": 3,
    "kinds": {
      "added": 1,
      "removed": 1,
      "major": 1,
      "minor": 0,
      "patch": 1,
      "downgrade": 1,
      "changed": 0,
      "integrity": 1,
      "source": 0
    }
  },
  "changes": [
    {
      "kind": "integrity_changed",
      "severity": "critical",
      "package": "left-pad",
      "before": "sha512-left-pad-old",
      "after": "sha512-left-pad-new",
      "detail": "checksum changed for version 1.3.0",
      "dev": false,
      "optional": false
    }
  ]
}
```

`changes` contains every finding in deterministic order; the example above shows
one entry for brevity. `before` and `after` are versions for version changes and
checksums or sources for metadata changes. `dev` and `optional` come from the
lockfile where it records them.

## Ignoring noisy packages

Some lockfiles churn on packages you do not review. Ignore exact names:

```sh
locklens diff old.lock new.lock --ignore '@types/node' --ignore some-cli
```

Ignored packages are excluded from the summary and from `--fail-on` decisions.
The JSON report lists them under `ignored`.

## Use with hix

LockLens is part of the [hix tools](https://github.com/hix-pro#readme) suite.
[`hix`](https://github.com/hix-pro/hix) runs the suite's CLI tools with one table
and one exit code. LockLens needs two files, so it cannot be auto-detected; add
it to `.hix.toml` explicitly:

```toml
[checks.locklens]
command = "locklens diff .locklens/base/package-lock.json package-lock.json --fail-on warning --json"
enabled = true
```

Generate the base lockfile in CI and commit the config when you want the plan to
be reproducible.

## Limits

- **No vulnerability data.** LockLens does not query advisory databases. Pair it
  with `npm audit`, `cargo audit`, `pip-audit`, or OSV.
- **No license metadata.** Lockfiles generally do not contain licenses; use a
  license scanner for that.
- **Checksum changes are signals, not proof.** A changed checksum can be a
  legitimate rebuild, a mirror, or a compromised artifact. LockLens tells you
  where to look; it does not decide whether the artifact is safe.
- **Cross-format comparisons skip metadata.** npm and pnpm store integrity
  differently, so comparing one to the other compares versions only.
- **No dependency paths or reachability.** The report says which package changed,
  not which direct dependency pulled it in.
- **Local workspace packages are skipped** in Cargo, uv, and Poetry output, and
  npm `link:` entries are skipped, because their versions follow the project.
- **Non-semver versions are reported as changed**, not ranked as major or minor.
- **LockLens never modifies a lockfile.** It has no `--fix` by design.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo run -- diff examples/before/package-lock.json examples/after/package-lock.json
```

The integration tests build the real binary and overwrite nothing. Parser tests
use inline fixtures for each supported format.

## Related tools

- [EnvAudit](https://github.com/hix-pro/env-audit) — environment variable drift and committed secrets.
- [Migration Guard](https://github.com/hix-pro/migration-guard) — dangerous SQL migrations, statically.
- [API Watch](https://github.com/hix-pro/api-watch) — breaking API contract changes.
- [TypeShift](https://github.com/hix-pro/typeshift) — TypeScript declaration API compatibility.
- [ActionLens](https://github.com/hix-pro/action-lens) — GitHub Actions workflow review in the browser.
- [ComposeMap](https://github.com/hix-pro/compose-map) — Docker Compose red flags in the browser.
- [RepoRace](https://github.com/hix-pro/reporace) — repository momentum, not star counts.
- [OpenCode Telegram](https://github.com/hix-pro/opencode-telegram) — remote-control a coding agent from Telegram.

## License

MIT. See [LICENSE](LICENSE).
