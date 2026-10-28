use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::diff::{self, Diff};
use crate::lockfile;
use crate::report::{self, TextOptions};

#[derive(Debug, Parser)]
#[command(
    name = "locklens",
    version,
    about = "See what actually changed between two lockfiles.",
    long_about = "LockLens compares two lockfiles by package name and reports upgrades, \
                  downgrades, additions, removals, and supply-chain signals such as a changed \
                  checksum or a different resolved source. It reads files only: no registry \
                  calls, no account, no telemetry.",
    after_help = "Examples:\n  locklens diff package-lock.json.old package-lock.json\n  locklens diff Cargo.lock.old Cargo.lock --markdown\n  locklens diff old.lock new.lock --fail-on warning\n\nExit codes:\n  0  success or report-only run with changes\n  1  usage, I/O, or parse error\n  2  findings matched --fail-on"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two lockfiles and report the package-level changes.
    Diff(DiffArgs),
}

#[derive(Debug, Args)]
struct DiffArgs {
    /// Old lockfile.
    old: PathBuf,

    /// New lockfile.
    new: PathBuf,

    /// Emit a stable JSON document.
    #[arg(long, conflicts_with_all = ["markdown", "sarif"])]
    json: bool,

    /// Emit a Markdown report.
    #[arg(long, conflicts_with_all = ["json", "sarif"])]
    markdown: bool,

    /// Emit a SARIF 2.1.0 report for GitHub code scanning.
    #[arg(long, conflicts_with_all = ["json", "markdown"])]
    sarif: bool,

    /// Ignore a package by exact name. Repeat for several packages.
    #[arg(long = "ignore", short = 'i', value_name = "PACKAGE")]
    ignore: Vec<String>,

    /// Exit with code 2 when findings at or above this severity are present.
    #[arg(long, value_enum)]
    fail_on: Option<FailOn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    /// Fail only when a checksum changed for an unchanged version.
    Critical,
    /// Fail on critical or source changes.
    Error,
    /// Fail on critical, source, downgrade, and breaking-range upgrades.
    Warning,
    /// Fail on any finding, including additions and patch upgrades.
    Any,
}

impl FailOn {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Any => "any",
        }
    }
}

/// Parses arguments, compares the lockfiles, renders output, and returns the
/// process exit code.
pub fn run() -> Result<u8> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let display_only = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            if let Err(print_error) = error.print() {
                if is_early_reader_exit(&print_error) {
                    // The reader went away (`locklens --help | head -1`);
                    // treat it as normal termination.
                    return Ok(0);
                }
                return Err(print_error).context("failed to print help");
            }
            return Ok(if display_only { 0 } else { 1 });
        }
    };

    let Command::Diff(args) = cli.command;
    let old = lockfile::load(&args.old)?;
    let new = lockfile::load(&args.new)?;
    let diff = diff::compare(
        &args.old.display().to_string(),
        &old,
        &args.new.display().to_string(),
        &new,
        &args.ignore,
    );
    let failing = args
        .fail_on
        .is_some_and(|threshold| threshold_met(&diff, threshold));

    let output = if args.json {
        report::render_json(&diff)?
    } else if args.markdown {
        report::render_markdown(&diff)
    } else if args.sarif {
        report::render_sarif(&diff)?
    } else {
        report::render_text(
            &diff,
            TextOptions {
                color: report::color_enabled(),
                fail_on: args.fail_on.map(FailOn::as_str),
                failing,
            },
        )
    };

    write_stdout(&output)?;
    Ok(if failing { 2 } else { 0 })
}

pub fn threshold_met(diff: &Diff, threshold: FailOn) -> bool {
    match threshold {
        FailOn::Critical => diff.summary.critical > 0,
        FailOn::Error => diff.summary.critical + diff.summary.error > 0,
        FailOn::Warning => diff.summary.critical + diff.summary.error + diff.summary.warning > 0,
        FailOn::Any => !diff.changes.is_empty(),
    }
}

fn write_stdout(output: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = stdout.write_all(output.as_bytes()) {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(error).context("failed to write output");
    }
    stdout.flush().context("failed to flush output")?;
    Ok(())
}

fn is_early_reader_exit(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{KindCounts, LockfileInfo, Summary};
    use crate::model::{Finding, FindingKind, LockFormat, Severity};

    fn diff_with(severity: Option<Severity>) -> Diff {
        let changes = severity
            .map(|severity| {
                vec![Finding::new(
                    FindingKind::PatchUpgrade,
                    severity,
                    "foo",
                    "patch upgrade",
                )]
            })
            .unwrap_or_default();
        let count = |target: Severity| {
            changes
                .iter()
                .filter(|change| change.severity == target)
                .count()
        };
        Diff {
            old: LockfileInfo {
                path: "old".to_owned(),
                format: LockFormat::Npm,
                lockfile_version: None,
                packages: 1,
            },
            new: LockfileInfo {
                path: "new".to_owned(),
                format: LockFormat::Npm,
                lockfile_version: None,
                packages: 1,
            },
            ignored: Vec::new(),
            summary: Summary {
                total: changes.len(),
                packages_changed: changes.len(),
                unchanged: 0,
                critical: count(Severity::Critical),
                error: count(Severity::Error),
                warning: count(Severity::Warning),
                info: count(Severity::Info),
                kinds: KindCounts::default(),
            },
            changes,
        }
    }

    #[test]
    fn thresholds_are_inclusive_of_higher_severities() {
        let critical = diff_with(Some(Severity::Critical));
        assert!(threshold_met(&critical, FailOn::Critical));
        assert!(threshold_met(&critical, FailOn::Error));
        assert!(threshold_met(&critical, FailOn::Warning));
        assert!(threshold_met(&critical, FailOn::Any));

        let info = diff_with(Some(Severity::Info));
        assert!(!threshold_met(&info, FailOn::Critical));
        assert!(!threshold_met(&info, FailOn::Error));
        assert!(!threshold_met(&info, FailOn::Warning));
        assert!(threshold_met(&info, FailOn::Any));

        assert!(!threshold_met(&diff_with(None), FailOn::Any));
    }

    #[test]
    fn threshold_labels_match_the_cli() {
        assert_eq!(FailOn::Critical.as_str(), "critical");
        assert_eq!(FailOn::Any.as_str(), "any");
    }
}
