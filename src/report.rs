use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;

use crate::diff::{Diff, Summary};
use crate::model::{Finding, FindingKind, Severity};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    tool: ToolInfo,
    old: &'a crate::diff::LockfileInfo,
    new: &'a crate::diff::LockfileInfo,
    ignored: &'a [String],
    summary: &'a Summary,
    changes: &'a [Finding],
}

#[derive(Debug, Clone, Serialize)]
struct ToolInfo {
    name: &'static str,
    version: &'static str,
}

pub fn render_json(diff: &Diff) -> Result<String> {
    let report = JsonReport {
        schema_version: SCHEMA_VERSION,
        tool: ToolInfo {
            name: "locklens",
            version: env!("CARGO_PKG_VERSION"),
        },
        old: &diff.old,
        new: &diff.new,
        ignored: &diff.ignored,
        summary: &diff.summary,
        changes: &diff.changes,
    };
    let mut output = serde_json::to_string_pretty(&report)?;
    output.push('\n');
    Ok(output)
}

pub struct TextOptions<'a> {
    pub color: bool,
    pub fail_on: Option<&'a str>,
    pub failing: bool,
}

pub fn render_text(diff: &Diff, options: TextOptions<'_>) -> String {
    let style = Style::new(options.color);
    let mut out = String::new();

    let _ = writeln!(
        out,
        "{}{} {}",
        style.bold("LockLens"),
        style.dim(" ·"),
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        out,
        "{} {}  {} · {} packages",
        style.dim("Old"),
        diff.old.path,
        diff.old.format.label(),
        diff.old.packages
    );
    let _ = writeln!(
        out,
        "{} {}  {} · {} packages",
        style.dim("New"),
        diff.new.path,
        diff.new.format.label(),
        diff.new.packages
    );
    if !diff.ignored.is_empty() {
        let _ = writeln!(out, "{} {}", style.dim("Ignored"), diff.ignored.join(", "));
    }
    out.push('\n');

    if diff.changes.is_empty() {
        out.push_str("No package changes.\n");
    } else {
        for severity in Severity::ALL {
            let group: Vec<&Finding> = diff
                .changes
                .iter()
                .filter(|change| change.severity == severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            let heading = format!("{} ({})", severity.label(), group.len());
            let _ = writeln!(out, "{}", style.severity(severity, &heading));
            for finding in group {
                push_text_finding(&mut out, finding, &style);
            }
            out.push('\n');
        }
    }

    let _ = writeln!(out, "{}", style.bold(&format_summary(&diff.summary)));
    let result = match options.fail_on {
        None => {
            "Result: report only (use --fail-on critical|error|warning|any to gate CI)".to_owned()
        }
        Some(threshold) if options.failing => {
            format!("Result: fail (--fail-on {threshold} matched findings)")
        }
        Some(threshold) => {
            format!("Result: pass (--fail-on {threshold} found no matching findings)")
        }
    };
    let _ = writeln!(out, "{result}");

    out
}

fn push_text_finding(out: &mut String, finding: &Finding, style: &Style) {
    let _ = writeln!(
        out,
        "  {}  {}{}",
        style.dim(&format!("{:<11}", finding.kind.as_str())),
        style.bold(&finding.package),
        flags_suffix(finding, style)
    );

    let has_versions = finding.before.is_some() || finding.after.is_some();
    if has_versions
        && !matches!(
            finding.kind,
            FindingKind::IntegrityChanged | FindingKind::SourceChanged
        )
    {
        let change = match (&finding.before, &finding.after) {
            (Some(before), Some(after)) => format!("{before} → {after}"),
            (None, Some(after)) => after.clone(),
            (Some(before), None) => before.clone(),
            (None, None) => String::new(),
        };
        let _ = writeln!(out, "    {change} ({})", finding.detail);
    } else {
        let _ = writeln!(out, "    {}", finding.detail);
    }

    if matches!(
        finding.kind,
        FindingKind::IntegrityChanged | FindingKind::SourceChanged
    ) {
        if let Some(before) = &finding.before {
            let _ = writeln!(out, "    {} {before}", style.dim("old:"));
        }
        if let Some(after) = &finding.after {
            let _ = writeln!(out, "    {} {after}", style.dim("new:"));
        }
    }
}

fn flags_suffix(finding: &Finding, style: &Style) -> String {
    let mut flags = Vec::new();
    if finding.dev {
        flags.push("dev");
    }
    if finding.optional {
        flags.push("optional");
    }
    if flags.is_empty() {
        String::new()
    } else {
        style.dim(&format!(" ({})", flags.join(", ")))
    }
}

fn format_summary(summary: &Summary) -> String {
    if summary.total == 0 {
        return format!(
            "Summary: no package changes ({} unchanged)",
            summary.unchanged
        );
    }

    let mut counts = Vec::new();
    for (count, label) in [
        (summary.critical, "critical"),
        (summary.error, "error"),
        (summary.warning, "warning"),
        (summary.info, "info"),
    ] {
        if count > 0 {
            counts.push(format!(
                "{count} {}",
                if count == 1 { label } else { plural(label) }
            ));
        }
    }
    let findings = if summary.total == 1 {
        "1 finding".to_owned()
    } else {
        format!("{} findings", summary.total)
    };
    let mut packages = if summary.packages_changed == 1 {
        "1 package changed".to_owned()
    } else {
        format!("{} packages changed", summary.packages_changed)
    };
    if summary.unchanged > 0 {
        packages.push_str(&format!(", {} unchanged", summary.unchanged));
    }
    format!("Summary: {packages} · {findings} ({})", counts.join(", "))
}

fn plural(label: &str) -> &str {
    match label {
        "error" => "errors",
        "warning" => "warnings",
        _ => label,
    }
}

pub fn render_markdown(diff: &Diff) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# LockLens report\n");
    let _ = writeln!(out, "| | Lockfile | Format | Packages |");
    let _ = writeln!(out, "| --- | --- | --- | --- |");
    let _ = writeln!(
        out,
        "| Old | `{}` | {} | {} |",
        diff.old.path,
        diff.old.format.label(),
        diff.old.packages
    );
    let _ = writeln!(
        out,
        "| New | `{}` | {} | {} |\n",
        diff.new.path,
        diff.new.format.label(),
        diff.new.packages
    );
    let _ = writeln!(out, "**{}**\n", format_summary(&diff.summary));

    if diff.changes.is_empty() {
        out.push_str("No package changes.\n");
        return out;
    }

    for severity in Severity::ALL {
        let group: Vec<&Finding> = diff
            .changes
            .iter()
            .filter(|change| change.severity == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "## {} ({})\n",
            title_case(severity.label()),
            group.len()
        );
        for finding in group {
            let _ = writeln!(
                out,
                "- **`{}`** — {}{}",
                finding.package,
                finding.detail,
                flags_markdown(finding)
            );
            push_markdown_versions(&mut out, finding);
        }
        out.push('\n');
    }
    out
}

fn push_markdown_versions(out: &mut String, finding: &Finding) {
    if matches!(
        finding.kind,
        FindingKind::IntegrityChanged | FindingKind::SourceChanged
    ) {
        if let Some(before) = &finding.before {
            let _ = writeln!(out, "  - old: `{before}`");
        }
        if let Some(after) = &finding.after {
            let _ = writeln!(out, "  - new: `{after}`");
        }
        return;
    }

    match (&finding.before, &finding.after) {
        (Some(before), Some(after)) => {
            let _ = writeln!(out, "  - `{before}` → `{after}`");
        }
        (None, Some(after)) => {
            let _ = writeln!(out, "  - new: `{after}`");
        }
        (Some(before), None) => {
            let _ = writeln!(out, "  - removed: `{before}`");
        }
        (None, None) => {}
    }
}

fn flags_markdown(finding: &Finding) -> String {
    let mut flags = Vec::new();
    if finding.dev {
        flags.push("dev");
    }
    if finding.optional {
        flags.push("optional");
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!(" _({})_", flags.join(", "))
    }
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
        None => String::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Debug, Clone, Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Debug, Clone, Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: &'static str,
    information_uri: &'static str,
    version: &'static str,
    rules: Vec<SarifRule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    id: &'static str,
    name: &'static str,
    short_description: SarifText,
    help_uri: &'static str,
    default_configuration: SarifLevel,
}

#[derive(Debug, Clone, Serialize)]
struct SarifText {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct SarifLevel {
    level: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: &'static str,
    rule_index: usize,
    level: &'static str,
    message: SarifText,
    locations: Vec<SarifLocation>,
    properties: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
    logical_locations: Vec<SarifLogicalLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Debug, Clone, Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLogicalLocation {
    name: String,
    fully_qualified_name: String,
    kind: &'static str,
}

const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const SARIF_HELP_URI: &str = "https://github.com/hix-pro/locklens#what-counts-as-a-finding";

const SARIF_RULES: [(&str, &str, &str, &str); 9] = [
    (
        "locklens/added",
        "PackageAdded",
        "A package was added to the lockfile.",
        "note",
    ),
    (
        "locklens/removed",
        "PackageRemoved",
        "A package was removed from the lockfile.",
        "note",
    ),
    (
        "locklens/major-upgrade",
        "MajorUpgrade",
        "A package moved across a breaking version range.",
        "warning",
    ),
    (
        "locklens/minor-upgrade",
        "MinorUpgrade",
        "A package received a minor upgrade.",
        "note",
    ),
    (
        "locklens/patch-upgrade",
        "PatchUpgrade",
        "A package received a patch upgrade.",
        "note",
    ),
    (
        "locklens/downgrade",
        "Downgrade",
        "A package resolved to an older version.",
        "warning",
    ),
    (
        "locklens/version-changed",
        "VersionsChanged",
        "The set of resolved versions changed without a single clear upgrade.",
        "warning",
    ),
    (
        "locklens/integrity-changed",
        "IntegrityChanged",
        "An unchanged package version resolved to a different checksum.",
        "error",
    ),
    (
        "locklens/source-changed",
        "SourceChanged",
        "An unchanged package version resolved from a different source.",
        "error",
    ),
];

fn rule_for(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Added => "locklens/added",
        FindingKind::Removed => "locklens/removed",
        FindingKind::MajorUpgrade => "locklens/major-upgrade",
        FindingKind::MinorUpgrade => "locklens/minor-upgrade",
        FindingKind::PatchUpgrade => "locklens/patch-upgrade",
        FindingKind::Downgrade => "locklens/downgrade",
        FindingKind::VersionChanged => "locklens/version-changed",
        FindingKind::IntegrityChanged => "locklens/integrity-changed",
        FindingKind::SourceChanged => "locklens/source-changed",
    }
}

fn rule_index(rule_id: &str) -> usize {
    SARIF_RULES
        .iter()
        .position(|(id, ..)| *id == rule_id)
        .unwrap_or(0)
}

fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical | Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

fn sarif_uri(path: &str) -> String {
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
    } else {
        Some(path.to_path_buf())
    };
    let Some(path) = relative else {
        return path.to_string_lossy().replace('\\', "/");
    };
    let value = path.to_string_lossy().replace('\\', "/");
    value.strip_prefix("./").unwrap_or(&value).to_owned()
}

/// Prefers a path that resolves inside the repository. A base-branch lockfile
/// copied to `/tmp` is not a repository file, so a removed package falls back
/// to the new lockfile instead of producing an annotation GitHub cannot place.
fn sarif_path(chosen: &str, fallback: &str) -> String {
    if is_repository_path(chosen) {
        chosen.to_owned()
    } else if is_repository_path(fallback) {
        fallback.to_owned()
    } else {
        chosen.to_owned()
    }
}

fn is_repository_path(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute() {
        return std::env::current_dir()
            .ok()
            .is_some_and(|cwd| path.strip_prefix(cwd).is_ok());
    }
    !path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn sarif_fully_qualified_name(finding: &Finding) -> String {
    if matches!(
        finding.kind,
        FindingKind::IntegrityChanged | FindingKind::SourceChanged
    ) {
        return finding.package.clone();
    }
    match (&finding.before, &finding.after) {
        (Some(before), Some(after)) if before != after => {
            format!("{}@{before} -> {after}", finding.package)
        }
        (None, Some(after)) => format!("{}@{after}", finding.package),
        (Some(before), None) => format!("{}@{before}", finding.package),
        _ => finding.package.clone(),
    }
}

/// SARIF 2.1.0 output for GitHub code scanning. Results are file-level
/// annotations on the lockfile; removed packages point at the old file.
pub fn render_sarif(diff: &Diff) -> Result<String> {
    let rules = SARIF_RULES
        .iter()
        .map(|(id, name, description, level)| SarifRule {
            id,
            name,
            short_description: SarifText {
                text: (*description).to_owned(),
            },
            help_uri: SARIF_HELP_URI,
            default_configuration: SarifLevel { level },
        })
        .collect();

    let results = diff
        .changes
        .iter()
        .map(|finding| {
            let rule_id = rule_for(finding.kind);
            let path = if finding.kind == FindingKind::Removed {
                sarif_path(&diff.old.path, &diff.new.path)
            } else {
                sarif_path(&diff.new.path, &diff.old.path)
            };
            let message = match (&finding.before, &finding.after) {
                (Some(before), Some(after)) => format!(
                    "{}: {} ({} → {})\n\n{}",
                    finding.package, finding.detail, before, after, finding.package
                ),
                (None, Some(after)) => format!(
                    "{}: {} ({})\n\nPackage: {}",
                    finding.package, finding.detail, after, finding.package
                ),
                (Some(before), None) => format!(
                    "{}: {} (was {})\n\nPackage: {}",
                    finding.package, finding.detail, before, finding.package
                ),
                (None, None) => format!("{}: {}", finding.package, finding.detail),
            };

            SarifResult {
                rule_id,
                rule_index: rule_index(rule_id),
                level: sarif_level(finding.severity),
                message: SarifText { text: message },
                locations: vec![SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation {
                            uri: sarif_uri(&path),
                        },
                        region: SarifRegion { start_line: 1 },
                    },
                    logical_locations: vec![SarifLogicalLocation {
                        name: finding.package.clone(),
                        fully_qualified_name: sarif_fully_qualified_name(finding),
                        kind: "package",
                    }],
                }],
                properties: json!({
                    "package": finding.package,
                    "detail": finding.detail,
                    "before": finding.before,
                    "after": finding.after,
                    "dev": finding.dev,
                    "optional": finding.optional,
                    "oldFormat": diff.old.format.as_str(),
                    "newFormat": diff.new.format.as_str(),
                }),
            }
        })
        .collect();

    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "LockLens",
                    information_uri: "https://github.com/hix-pro/locklens",
                    version: env!("CARGO_PKG_VERSION"),
                    rules,
                },
            },
            results,
        }],
    };

    let mut output = serde_json::to_string_pretty(&log)?;
    output.push('\n');
    Ok(output)
}

pub fn color_enabled() -> bool {
    color_enabled_for(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var("TERM").ok().as_deref(),
    )
}

fn color_enabled_for(is_terminal: bool, no_color: bool, term: Option<&str>) -> bool {
    is_terminal && !no_color && !matches!(term, Some("dumb"))
}

struct Style {
    enabled: bool,
}

impl Style {
    fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    fn paint(&self, code: &str, value: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{value}\x1b[0m")
        } else {
            value.to_owned()
        }
    }

    fn bold(&self, value: &str) -> String {
        self.paint("1", value)
    }

    fn dim(&self, value: &str) -> String {
        self.paint("2", value)
    }

    fn severity(&self, severity: Severity, value: &str) -> String {
        match severity {
            Severity::Critical | Severity::Error => self.paint("1;31", value),
            Severity::Warning => self.paint("1;33", value),
            Severity::Info => self.paint("1;36", value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{KindCounts, LockfileInfo, Summary};
    use crate::model::LockFormat;

    fn diff_with(changes: Vec<Finding>) -> Diff {
        Diff {
            old: LockfileInfo {
                path: "old/package-lock.json".to_owned(),
                format: LockFormat::Npm,
                lockfile_version: Some("3".to_owned()),
                packages: 4,
            },
            new: LockfileInfo {
                path: "new/package-lock.json".to_owned(),
                format: LockFormat::Npm,
                lockfile_version: Some("3".to_owned()),
                packages: 5,
            },
            ignored: Vec::new(),
            summary: Summary {
                total: changes.len(),
                packages_changed: changes.len(),
                unchanged: 2,
                critical: changes
                    .iter()
                    .filter(|change| change.severity == Severity::Critical)
                    .count(),
                error: changes
                    .iter()
                    .filter(|change| change.severity == Severity::Error)
                    .count(),
                warning: changes
                    .iter()
                    .filter(|change| change.severity == Severity::Warning)
                    .count(),
                info: changes
                    .iter()
                    .filter(|change| change.severity == Severity::Info)
                    .count(),
                kinds: KindCounts::default(),
            },
            changes,
        }
    }

    #[test]
    fn json_has_a_stable_top_level_schema() {
        let diff = diff_with(vec![
            Finding::new(FindingKind::Added, Severity::Info, "kleur", "new package").after("4.1.5"),
        ]);
        let value: serde_json::Value = serde_json::from_str(&render_json(&diff).unwrap()).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["tool"]["name"], "locklens");
        assert_eq!(value["old"]["format"], "npm");
        assert_eq!(value["new"]["packages"], 5);
        assert_eq!(value["changes"][0]["kind"], "added");
        assert_eq!(value["changes"][0]["after"], "4.1.5");
        assert_eq!(value["summary"]["total"], 1);
    }

    #[test]
    fn text_output_groups_by_severity_and_shows_the_threshold_result() {
        let diff = diff_with(vec![
            Finding::new(
                FindingKind::IntegrityChanged,
                Severity::Critical,
                "left-pad",
                "checksum changed for version 1.3.0",
            )
            .before("sha512-old")
            .after("sha512-new"),
            Finding::new(
                FindingKind::MajorUpgrade,
                Severity::Warning,
                "vite",
                "major upgrade",
            )
            .before("5.4.0")
            .after("6.0.0"),
        ]);
        let text = render_text(
            &diff,
            TextOptions {
                color: false,
                fail_on: Some("warning"),
                failing: true,
            },
        );

        assert!(text.contains("critical (1)"));
        assert!(text.contains("integrity"));
        assert!(text.contains("left-pad"));
        assert!(text.contains("old: sha512-old"));
        assert!(text.contains("warning (1)"));
        assert!(text.contains("5.4.0 → 6.0.0 (major upgrade)"));
        assert!(text.contains("Result: fail (--fail-on warning matched findings)"));
    }

    #[test]
    fn markdown_is_deterministic_and_contains_the_summary() {
        let diff = diff_with(vec![
            Finding::new(
                FindingKind::Downgrade,
                Severity::Warning,
                "typescript",
                "downgrade",
            )
            .before("5.6.0")
            .after("5.5.0"),
        ]);
        let markdown = render_markdown(&diff);

        assert!(markdown.starts_with("# LockLens report"));
        assert!(markdown.contains("| Old | `old/package-lock.json` | npm | 4 |"));
        assert!(markdown.contains("## Warning (1)"));
        assert!(markdown.contains("`5.6.0` → `5.5.0`"));
    }

    #[test]
    fn sarif_maps_severity_to_levels_and_points_at_lockfiles() {
        let diff = diff_with(vec![
            Finding::new(
                FindingKind::IntegrityChanged,
                Severity::Critical,
                "left-pad",
                "checksum changed for version 1.3.0",
            )
            .before("sha512-old")
            .after("sha512-new"),
            Finding::new(
                FindingKind::Removed,
                Severity::Info,
                "chalk",
                "removed from the lockfile",
            )
            .before("4.0.0"),
        ]);
        let value: serde_json::Value = serde_json::from_str(&render_sarif(&diff).unwrap()).unwrap();
        let results = value["runs"][0]["results"].as_array().unwrap();

        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["tool"]["driver"]["name"], "LockLens");
        assert_eq!(
            value["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
        assert_eq!(results[0]["ruleId"], "locklens/integrity-changed");
        assert_eq!(results[0]["level"], "error");
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "new/package-lock.json"
        );
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["region"]["startLine"],
            1
        );
        assert_eq!(
            results[0]["locations"][0]["logicalLocations"][0]["fullyQualifiedName"],
            "left-pad"
        );
        assert_eq!(results[1]["ruleId"], "locklens/removed");
        assert_eq!(
            results[1]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "old/package-lock.json"
        );
        assert_eq!(results[1]["level"], "note");
    }

    #[test]
    fn sarif_removed_package_falls_back_to_the_new_lockfile_outside_the_repo() {
        let mut diff = diff_with(vec![
            Finding::new(
                FindingKind::Removed,
                Severity::Info,
                "chalk",
                "removed from the lockfile",
            )
            .before("4.0.0"),
        ]);
        diff.old.path = "/tmp/package-lock.json.base".to_owned();
        let value: serde_json::Value = serde_json::from_str(&render_sarif(&diff).unwrap()).unwrap();
        let uri = &value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
            ["uri"];

        assert_eq!(uri, "new/package-lock.json");
    }

    #[test]
    fn summary_grammar_handles_zero_and_multiple_findings() {
        let mut diff = diff_with(Vec::new());
        diff.summary.total = 0;
        diff.summary.packages_changed = 0;
        assert_eq!(
            format_summary(&diff.summary),
            "Summary: no package changes (2 unchanged)"
        );

        let diff = diff_with(vec![
            Finding::new(FindingKind::SourceChanged, Severity::Error, "a", "error"),
            Finding::new(FindingKind::Downgrade, Severity::Warning, "b", "downgrade"),
        ]);
        assert!(format_summary(&diff.summary).contains("1 error, 1 warning"));
    }
}
