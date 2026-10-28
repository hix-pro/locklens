# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-10-28

### Added
- `locklens diff <old> <new>` compares npm, pnpm, Yarn classic, Yarn Berry, Cargo, uv, and Poetry lockfiles.
- Classifies additions, removals, major/minor/patch upgrades, downgrades, multi-version changes, checksum changes, and resolved-source changes.
- `--json`, `--markdown`, and `--sarif` reports with deterministic ordering.
- `--fail-on critical|error|warning|any` for CI gating, report-only by default.
- `--ignore <package>` to filter noisy package names.
- Composite GitHub Action that downloads a prebuilt binary, uploads SARIF by default, and preserves the comparison exit code.
- Prebuilt release binaries for Linux x86_64, macOS Apple Silicon, and Windows x86_64.
