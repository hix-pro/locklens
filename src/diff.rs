use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use serde::Serialize;

use crate::model::{Finding, FindingKind, LockFormat, Lockfile, Package, Severity};

#[derive(Debug, Clone, Serialize)]
pub struct LockfileInfo {
    pub path: String,
    pub format: LockFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile_version: Option<String>,
    pub packages: usize,
}

impl LockfileInfo {
    fn new(path: &str, lockfile: &Lockfile) -> Self {
        Self {
            path: path.to_owned(),
            format: lockfile.format,
            lockfile_version: lockfile.lockfile_version.clone(),
            packages: lockfile.package_count(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KindCounts {
    pub added: usize,
    pub removed: usize,
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    pub downgrade: usize,
    pub changed: usize,
    pub integrity: usize,
    pub source: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total: usize,
    pub packages_changed: usize,
    pub unchanged: usize,
    pub critical: usize,
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    pub kinds: KindCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diff {
    pub old: LockfileInfo,
    pub new: LockfileInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
    pub summary: Summary,
    pub changes: Vec<Finding>,
}

/// Compares two parsed lockfiles by package name, not by file order. Metadata
/// checks (checksum and source) only run when both files use the same format;
/// cross-format comparisons would otherwise report every package as changed.
pub fn compare(
    old_path: &str,
    old: &Lockfile,
    new_path: &str,
    new: &Lockfile,
    ignore: &[String],
) -> Diff {
    let ignored: BTreeSet<&str> = ignore.iter().map(String::as_str).collect();
    let old_by_name = old.by_name();
    let new_by_name = new.by_name();
    let names: BTreeSet<&str> = old_by_name
        .keys()
        .chain(new_by_name.keys())
        .copied()
        .collect();

    let metadata_comparable = old.format == new.format;
    let mut changes = Vec::new();
    let mut changed_names = BTreeSet::new();

    for name in names {
        if ignored.contains(name) {
            continue;
        }
        let old_packages = old_by_name.get(name).cloned().unwrap_or_default();
        let new_packages = new_by_name.get(name).cloned().unwrap_or_default();
        let before = changes.len();
        compare_package(
            name,
            &old_packages,
            &new_packages,
            metadata_comparable,
            &mut changes,
        );
        if changes.len() > before {
            changed_names.insert(name.to_owned());
        }
    }

    changes.sort_by(compare_findings);
    let summary = summarize(
        &changes,
        &changed_names,
        &old_by_name,
        &new_by_name,
        &ignored,
    );

    Diff {
        old: LockfileInfo::new(old_path, old),
        new: LockfileInfo::new(new_path, new),
        ignored: {
            let mut values: Vec<String> = ignored.iter().map(|name| (*name).to_owned()).collect();
            values.sort();
            values.dedup();
            values
        },
        summary,
        changes,
    }
}

fn compare_package(
    name: &str,
    old_packages: &[&Package],
    new_packages: &[&Package],
    metadata_comparable: bool,
    changes: &mut Vec<Finding>,
) {
    let old_versions: BTreeSet<&str> = old_packages
        .iter()
        .map(|package| package.version.as_str())
        .collect();
    let new_versions: BTreeSet<&str> = new_packages
        .iter()
        .map(|package| package.version.as_str())
        .collect();
    let (dev, optional) = package_flags(new_packages, old_packages);

    if old_packages.is_empty() {
        changes.push(
            Finding::new(
                FindingKind::Added,
                Severity::Info,
                name,
                if new_versions.len() == 1 {
                    "new package".to_owned()
                } else {
                    format!("new package with {} resolved versions", new_versions.len())
                },
            )
            .after(join_versions(&new_versions))
            .dev(dev)
            .optional(optional),
        );
        return;
    }

    if new_packages.is_empty() {
        changes.push(
            Finding::new(
                FindingKind::Removed,
                Severity::Info,
                name,
                if old_versions.len() == 1 {
                    "removed from the lockfile".to_owned()
                } else {
                    format!(
                        "removed from the lockfile ({} versions)",
                        old_versions.len()
                    )
                },
            )
            .before(join_versions(&old_versions))
            .dev(dev)
            .optional(optional),
        );
        return;
    }

    if metadata_comparable {
        for version in old_versions.intersection(&new_versions) {
            let old_package = old_packages
                .iter()
                .copied()
                .find(|package| package.version == *version);
            let new_package = new_packages
                .iter()
                .copied()
                .find(|package| package.version == *version);
            let (Some(old_package), Some(new_package)) = (old_package, new_package) else {
                continue;
            };
            let (dev, optional) = package_flags(&[new_package], &[old_package]);
            if old_package.integrities != new_package.integrities {
                changes.push(
                    Finding::new(
                        FindingKind::IntegrityChanged,
                        Severity::Critical,
                        name,
                        format!("checksum changed for version {version}"),
                    )
                    .before_optional(set_label(&old_package.integrities))
                    .after_optional(set_label(&new_package.integrities))
                    .dev(dev)
                    .optional(optional),
                );
            }
            if old_package.sources != new_package.sources {
                changes.push(
                    Finding::new(
                        FindingKind::SourceChanged,
                        Severity::Error,
                        name,
                        format!("resolved source changed for version {version}"),
                    )
                    .before_optional(set_label(&old_package.sources))
                    .after_optional(set_label(&new_package.sources))
                    .dev(dev)
                    .optional(optional),
                );
            }
        }
    }

    if old_versions == new_versions {
        return;
    }

    let (kind, severity, detail) = if old_versions.len() == 1 && new_versions.len() == 1 {
        let before = *old_versions.iter().next().expect("one version");
        let after = *new_versions.iter().next().expect("one version");
        classify_single(before, after)
    } else {
        let detail = if old_versions.is_subset(&new_versions) {
            "additional resolved version introduced"
        } else if new_versions.is_subset(&old_versions) {
            "a resolved version disappeared"
        } else {
            "resolved version set changed"
        };
        (
            FindingKind::VersionChanged,
            Severity::Warning,
            detail.to_owned(),
        )
    };

    changes.push(
        Finding::new(kind, severity, name, detail)
            .before(join_versions(&old_versions))
            .after(join_versions(&new_versions))
            .dev(dev)
            .optional(optional),
    );
}

fn classify_single(old: &str, new: &str) -> (FindingKind, Severity, String) {
    let (Some(old_version), Some(new_version)) = (parse_version(old), parse_version(new)) else {
        return (
            FindingKind::VersionChanged,
            Severity::Warning,
            "version changed (not semver)".to_owned(),
        );
    };

    if new_version < old_version {
        return (
            FindingKind::Downgrade,
            Severity::Warning,
            "downgrade".to_owned(),
        );
    }
    if new_version.major != old_version.major {
        return (
            FindingKind::MajorUpgrade,
            Severity::Warning,
            "major upgrade".to_owned(),
        );
    }
    if old_version.major == 0 && new_version.minor != old_version.minor {
        return (
            FindingKind::MajorUpgrade,
            Severity::Warning,
            "0.x minor upgrade (breaking range)".to_owned(),
        );
    }
    if new_version.minor != old_version.minor {
        return (
            FindingKind::MinorUpgrade,
            Severity::Info,
            "minor upgrade".to_owned(),
        );
    }
    if new_version.patch != old_version.patch {
        return (
            FindingKind::PatchUpgrade,
            Severity::Info,
            "patch upgrade".to_owned(),
        );
    }
    (
        FindingKind::VersionChanged,
        Severity::Info,
        "prerelease or build metadata changed".to_owned(),
    )
}

fn parse_version(value: &str) -> Option<Version> {
    Version::parse(value).ok().or_else(|| {
        let stripped = value.strip_prefix('v')?;
        Version::parse(stripped).ok()
    })
}

fn package_flags(primary: &[&Package], fallback: &[&Package]) -> (bool, bool) {
    let packages: Vec<&&Package> = primary.iter().chain(fallback.iter()).collect();
    if packages.is_empty() {
        return (false, false);
    }
    (
        packages.iter().all(|package| package.dev),
        packages.iter().all(|package| package.optional),
    )
}

fn join_versions(versions: &BTreeSet<&str>) -> String {
    versions.iter().copied().collect::<Vec<_>>().join(", ")
}

fn set_label(set: &BTreeSet<String>) -> Option<String> {
    if set.is_empty() {
        None
    } else {
        Some(set.iter().cloned().collect::<Vec<_>>().join(", "))
    }
}

fn summarize(
    changes: &[Finding],
    changed_names: &BTreeSet<String>,
    old_by_name: &BTreeMap<&str, Vec<&Package>>,
    new_by_name: &BTreeMap<&str, Vec<&Package>>,
    ignored: &BTreeSet<&str>,
) -> Summary {
    let mut kinds = KindCounts::default();
    let mut summary = Summary {
        total: changes.len(),
        packages_changed: changed_names.len(),
        unchanged: 0,
        critical: 0,
        error: 0,
        warning: 0,
        info: 0,
        kinds: KindCounts::default(),
    };

    for change in changes {
        match change.severity {
            Severity::Critical => summary.critical += 1,
            Severity::Error => summary.error += 1,
            Severity::Warning => summary.warning += 1,
            Severity::Info => summary.info += 1,
        }
        match change.kind {
            FindingKind::Added => kinds.added += 1,
            FindingKind::Removed => kinds.removed += 1,
            FindingKind::MajorUpgrade => kinds.major += 1,
            FindingKind::MinorUpgrade => kinds.minor += 1,
            FindingKind::PatchUpgrade => kinds.patch += 1,
            FindingKind::Downgrade => kinds.downgrade += 1,
            FindingKind::VersionChanged => kinds.changed += 1,
            FindingKind::IntegrityChanged => kinds.integrity += 1,
            FindingKind::SourceChanged => kinds.source += 1,
        }
    }

    let mut unchanged = 0;
    for name in old_by_name.keys() {
        if ignored.contains(*name) {
            continue;
        }
        if !new_by_name.contains_key(*name) || changed_names.contains(*name) {
            continue;
        }
        unchanged += 1;
    }
    summary.unchanged = unchanged;
    summary.kinds = kinds;
    summary
}

fn compare_findings(a: &Finding, b: &Finding) -> std::cmp::Ordering {
    a.severity
        .cmp(&b.severity)
        .then_with(|| a.kind.cmp(&b.kind))
        .then_with(|| a.package.cmp(&b.package))
        .then_with(|| a.before.cmp(&b.before))
        .then_with(|| a.after.cmp(&b.after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LockFormat;

    fn lockfile(format: LockFormat, packages: Vec<Package>) -> Lockfile {
        Lockfile::new(format, Some("1".to_owned()), packages)
    }

    fn pkg(name: &str, version: &str) -> Package {
        Package::new(name, version)
            .with_source(format!("https://registry.example/{name}/{version}"))
            .with_integrity(format!("sha512-{name}-{version}"))
    }

    fn diff(old: Vec<Package>, new: Vec<Package>) -> Diff {
        compare(
            "old.lock",
            &lockfile(LockFormat::Npm, old),
            "new.lock",
            &lockfile(LockFormat::Npm, new),
            &[],
        )
    }

    #[test]
    fn classifies_semver_upgrades_and_downgrades() {
        let result = diff(
            vec![
                pkg("patch", "1.2.3"),
                pkg("minor", "1.2.3"),
                pkg("major", "1.2.3"),
                pkg("zero", "0.1.0"),
                pkg("down", "2.0.0"),
            ],
            vec![
                pkg("patch", "1.2.4"),
                pkg("minor", "1.3.0"),
                pkg("major", "2.0.0"),
                pkg("zero", "0.2.0"),
                pkg("down", "1.9.0"),
            ],
        );

        let by_name: BTreeMap<_, _> = result
            .changes
            .iter()
            .map(|change| (change.package.as_str(), change))
            .collect();
        assert_eq!(by_name["patch"].kind, FindingKind::PatchUpgrade);
        assert_eq!(by_name["patch"].severity, Severity::Info);
        assert_eq!(by_name["minor"].kind, FindingKind::MinorUpgrade);
        assert_eq!(by_name["major"].kind, FindingKind::MajorUpgrade);
        assert_eq!(by_name["major"].severity, Severity::Warning);
        assert_eq!(by_name["zero"].kind, FindingKind::MajorUpgrade);
        assert!(by_name["zero"].detail.contains("0.x"));
        assert_eq!(by_name["down"].kind, FindingKind::Downgrade);
        assert_eq!(result.summary.kinds.major, 2);
        assert_eq!(result.summary.kinds.downgrade, 1);
    }

    #[test]
    fn flags_checksum_and_source_changes_for_unchanged_versions() {
        let old = Package::new("left-pad", "1.3.0")
            .with_source("https://registry.npmjs.org/left-pad")
            .with_integrity("sha512-old");
        let new = Package::new("left-pad", "1.3.0")
            .with_source("https://mirror.example/left-pad")
            .with_integrity("sha512-new");

        let result = diff(vec![old], vec![new]);
        assert_eq!(result.changes.len(), 2);
        assert_eq!(result.summary.critical, 1);
        assert_eq!(result.summary.error, 1);
        let integrity = result
            .changes
            .iter()
            .find(|change| change.kind == FindingKind::IntegrityChanged)
            .unwrap();
        assert_eq!(integrity.before.as_deref(), Some("sha512-old"));
        assert_eq!(integrity.after.as_deref(), Some("sha512-new"));
    }

    #[test]
    fn reports_additions_removals_and_unchanged_packages() {
        let result = diff(
            vec![pkg("stay", "1.0.0"), pkg("gone", "1.0.0")],
            vec![pkg("stay", "1.0.0"), pkg("new", "1.0.0")],
        );

        assert_eq!(result.summary.total, 2);
        assert_eq!(result.summary.packages_changed, 2);
        assert_eq!(result.summary.unchanged, 1);
        assert_eq!(result.summary.kinds.added, 1);
        assert_eq!(result.summary.kinds.removed, 1);
        assert_eq!(result.changes[0].package, "new");
        assert_eq!(result.changes[1].package, "gone");
    }

    #[test]
    fn reports_multi_version_changes_as_warnings() {
        let result = diff(
            vec![pkg("foo", "1.0.0"), pkg("foo", "2.0.0")],
            vec![pkg("foo", "1.0.0"), pkg("foo", "3.0.0")],
        );
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].kind, FindingKind::VersionChanged);
        assert_eq!(result.changes[0].severity, Severity::Warning);
        assert_eq!(result.changes[0].before.as_deref(), Some("1.0.0, 2.0.0"));
        assert_eq!(result.changes[0].after.as_deref(), Some("1.0.0, 3.0.0"));
    }

    #[test]
    fn ignored_packages_are_filtered_from_changes_and_unchanged_count() {
        let result = compare(
            "old.lock",
            &lockfile(LockFormat::Npm, vec![pkg("noisy", "1.0.0")]),
            "new.lock",
            &lockfile(LockFormat::Npm, vec![pkg("noisy", "2.0.0")]),
            &["noisy".to_owned()],
        );
        assert!(result.changes.is_empty());
        assert_eq!(result.summary.unchanged, 0);
        assert_eq!(result.ignored, ["noisy"]);
    }

    #[test]
    fn cross_format_comparisons_skip_metadata_checks() {
        let old = lockfile(
            LockFormat::Npm,
            vec![Package::new("foo", "1.0.0").with_integrity("sha512-a")],
        );
        let new = lockfile(
            LockFormat::Pnpm,
            vec![Package::new("foo", "1.0.0").with_integrity("sha512-b")],
        );
        let result = compare("old.lock", &old, "new.lock", &new, &[]);
        assert!(result.changes.is_empty());
    }

    #[test]
    fn findings_sort_by_severity_then_name() {
        let result = diff(
            vec![pkg("zeta", "1.0.0"), pkg("alpha", "1.0.0")],
            vec![pkg("zeta", "2.0.0"), pkg("alpha", "1.1.0")],
        );
        assert_eq!(result.changes[0].package, "zeta");
        assert_eq!(result.changes[0].severity, Severity::Warning);
        assert_eq!(result.changes[1].package, "alpha");
        assert_eq!(result.changes[1].severity, Severity::Info);
    }
}
