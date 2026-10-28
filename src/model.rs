use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

/// Ordered from most to least urgent; the order is used for report grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Error,
    Warning,
    Info,
}

impl Severity {
    pub const ALL: [Self; 4] = [Self::Critical, Self::Error, Self::Warning, Self::Info];

    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Added,
    Removed,
    MajorUpgrade,
    MinorUpgrade,
    PatchUpgrade,
    Downgrade,
    VersionChanged,
    IntegrityChanged,
    SourceChanged,
}

impl FindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::MajorUpgrade => "major",
            Self::MinorUpgrade => "minor",
            Self::PatchUpgrade => "patch",
            Self::Downgrade => "downgrade",
            Self::VersionChanged => "changed",
            Self::IntegrityChanged => "integrity",
            Self::SourceChanged => "source",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Removed => "Removed",
            Self::MajorUpgrade => "Major upgrade",
            Self::MinorUpgrade => "Minor upgrade",
            Self::PatchUpgrade => "Patch upgrade",
            Self::Downgrade => "Downgrade",
            Self::VersionChanged => "Versions changed",
            Self::IntegrityChanged => "Checksum changed",
            Self::SourceChanged => "Source changed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockFormat {
    Npm,
    Pnpm,
    YarnClassic,
    YarnBerry,
    Cargo,
    Uv,
    Poetry,
}

impl LockFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::YarnClassic => "yarn-classic",
            Self::YarnBerry => "yarn-berry",
            Self::Cargo => "cargo",
            Self::Uv => "uv",
            Self::Poetry => "poetry",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::YarnClassic => "Yarn classic",
            Self::YarnBerry => "Yarn Berry",
            Self::Cargo => "Cargo",
            Self::Uv => "uv",
            Self::Poetry => "Poetry",
        }
    }
}

/// One resolved package instance from a lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub sources: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub integrities: BTreeSet<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub dev: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub optional: bool,
}

impl Package {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            sources: BTreeSet::new(),
            integrities: BTreeSet::new(),
            dev: false,
            optional: false,
        }
    }

    pub fn add_source(&mut self, source: impl Into<String>) {
        let source = source.into();
        if !source.is_empty() {
            self.sources.insert(source);
        }
    }

    pub fn add_integrity(&mut self, integrity: impl Into<String>) {
        let integrity = integrity.into();
        if !integrity.is_empty() {
            self.integrities.insert(integrity);
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.add_source(source);
        self
    }

    pub fn with_integrity(mut self, integrity: impl Into<String>) -> Self {
        self.add_integrity(integrity);
        self
    }

    pub fn dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    pub fn optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }
}

pub fn is_false(value: &bool) -> bool {
    !*value
}

/// A parsed lockfile, deduplicated by package name and version.
#[derive(Debug, Clone, Serialize)]
pub struct Lockfile {
    pub format: LockFormat,
    pub lockfile_version: Option<String>,
    pub packages: Vec<Package>,
}

impl Lockfile {
    pub fn new(
        format: LockFormat,
        lockfile_version: Option<String>,
        packages: Vec<Package>,
    ) -> Self {
        Self {
            format,
            lockfile_version,
            packages: normalize(packages),
        }
    }

    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    pub fn by_name(&self) -> BTreeMap<&str, Vec<&Package>> {
        let mut groups: BTreeMap<&str, Vec<&Package>> = BTreeMap::new();
        for package in &self.packages {
            groups.entry(&package.name).or_default().push(package);
        }
        groups
    }
}

/// Merges duplicate `(name, version)` entries. npm and pnpm can resolve the
/// same package version through several paths, each with its own flags.
fn normalize(packages: Vec<Package>) -> Vec<Package> {
    let mut merged: BTreeMap<(String, String), Package> = BTreeMap::new();
    for package in packages {
        let key = (package.name.clone(), package.version.clone());
        match merged.entry(key) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(package);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                existing.sources.extend(package.sources);
                existing.integrities.extend(package.integrities);
                existing.dev |= package.dev;
                existing.optional |= package.optional;
            }
        }
    }
    merged.into_values().collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub package: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub detail: String,
    pub dev: bool,
    pub optional: bool,
}

impl Finding {
    pub fn new(
        kind: FindingKind,
        severity: Severity,
        package: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            severity,
            package: package.into(),
            before: None,
            after: None,
            detail: detail.into(),
            dev: false,
            optional: false,
        }
    }

    pub fn before(mut self, before: impl Into<String>) -> Self {
        self.before = Some(before.into());
        self
    }

    pub fn after(mut self, after: impl Into<String>) -> Self {
        self.after = Some(after.into());
        self
    }

    pub fn before_optional(mut self, before: Option<impl Into<String>>) -> Self {
        self.before = before.map(Into::into);
        self
    }

    pub fn after_optional(mut self, after: Option<impl Into<String>>) -> Self {
        self.after = after.map(Into::into);
        self
    }

    pub fn dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    pub fn optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_package_versions_are_merged() {
        let mut first = Package::new("foo", "1.0.0")
            .with_integrity("sha512-a")
            .dev(true);
        first
            .sources
            .insert("https://registry.npmjs.org/foo".to_owned());
        let second = Package::new("foo", "1.0.0")
            .with_integrity("sha512-b")
            .optional(true);

        let lockfile = Lockfile::new(LockFormat::Npm, Some("3".to_owned()), vec![first, second]);

        assert_eq!(lockfile.package_count(), 1);
        let package = &lockfile.packages[0];
        assert_eq!(package.integrities.len(), 2);
        assert!(package.dev);
        assert!(package.optional);
        assert_eq!(package.sources.len(), 1);
    }

    #[test]
    fn severity_and_kind_labels_are_stable() {
        assert_eq!(Severity::Critical.label(), "critical");
        assert_eq!(FindingKind::IntegrityChanged.as_str(), "integrity");
        assert_eq!(FindingKind::MajorUpgrade.label(), "Major upgrade");
        assert_eq!(LockFormat::YarnClassic.as_str(), "yarn-classic");
    }
}
