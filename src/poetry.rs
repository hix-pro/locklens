use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use toml::Value;

use crate::model::{LockFormat, Lockfile, Package};

/// Parses `poetry.lock`, including the older `[metadata.files]` layout and the
/// current per-package `files` array.
pub fn parse(text: &str) -> Result<Lockfile> {
    let value: Value = toml::from_str(text).context("invalid TOML")?;
    let metadata = value.get("metadata");
    let lockfile_version = metadata
        .and_then(|metadata| metadata.get("lock-version"))
        .and_then(scalar_string);

    let metadata_hashes = metadata_hashes(metadata);
    let Some(entries) = value.get("package").and_then(Value::as_array) else {
        bail!("no [[package]] entries found");
    };

    let mut packages = Vec::new();
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(version) = entry.get("version").and_then(Value::as_str) else {
            continue;
        };
        let source = entry.get("source");
        if is_local(source) {
            continue;
        }

        let mut package = Package::new(name, version);
        if let Some(source) = source.and_then(source_label) {
            package.add_source(source);
        }

        if let Some(files) = entry.get("files").and_then(Value::as_array) {
            for file in files {
                if let Some(hash) = file.get("hash").and_then(Value::as_str) {
                    package.add_integrity(hash);
                }
            }
        }
        if let Some(hashes) = metadata_hashes.get(name) {
            for hash in hashes {
                package.add_integrity(hash);
            }
        }

        let dev = entry
            .get("category")
            .and_then(Value::as_str)
            .is_some_and(|category| category == "dev")
            || entry
                .get("groups")
                .and_then(Value::as_array)
                .is_some_and(|groups| {
                    !groups
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|group| group == "main")
                });
        let optional = entry
            .get("optional")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        packages.push(package.dev(dev).optional(optional));
    }

    if packages.is_empty() {
        bail!("no packages found");
    }
    Ok(Lockfile::new(
        LockFormat::Poetry,
        lockfile_version,
        packages,
    ))
}

fn metadata_hashes(metadata: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    let mut hashes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(files) = metadata
        .and_then(|metadata| metadata.get("files"))
        .and_then(Value::as_table)
    else {
        return hashes;
    };
    for (name, entries) in files {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        let values = hashes.entry(name.clone()).or_default();
        for entry in entries {
            if let Some(hash) = entry.get("hash").and_then(Value::as_str) {
                values.push(hash.to_owned());
            }
        }
    }
    hashes
}

fn is_local(source: Option<&Value>) -> bool {
    source
        .and_then(|source| source.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "directory" | "file" | "path"))
}

fn source_label(source: &Value) -> Option<String> {
    let kind = source.get("type").and_then(Value::as_str)?;
    let url = source.get("url").and_then(Value::as_str);
    match kind {
        "git" => {
            let url = url?;
            let reference = source
                .get("resolved_reference")
                .and_then(Value::as_str)
                .or_else(|| source.get("reference").and_then(Value::as_str));
            Some(match reference {
                Some(reference) => format!("git:{url}#{reference}"),
                None => format!("git:{url}"),
            })
        }
        "url" | "legacy" => url.map(str::to_owned),
        _ => None,
    }
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Integer(number) => Some(number.to_string()),
        Value::Float(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_files_and_groups() {
        let text = r#"[metadata]
lock-version = "2.0"
python-versions = ">=3.11"

[[package]]
name = "attrs"
version = "24.2.0"
optional = false
groups = ["main"]
files = [
    {file = "attrs-24.2.0-py3-none-any.whl", hash = "sha256:aaa"},
]

[[package]]
name = "pytest"
version = "8.3.0"
optional = false
groups = ["dev"]
files = [
    {file = "pytest-8.3.0-py3-none-any.whl", hash = "sha256:bbb"},
]
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("2.0"));
        assert_eq!(lockfile.package_count(), 2);
        assert_eq!(lockfile.packages[0].name, "attrs");
        assert!(!lockfile.packages[0].dev);
        assert!(lockfile.packages[1].dev);
        assert!(lockfile.packages[1].integrities.contains("sha256:bbb"));
    }

    #[test]
    fn reads_legacy_metadata_files() {
        let text = r#"[metadata]
lock-version = "1.1"
[metadata.files]
attrs = [
    {file = "attrs-20.3.0.tar.gz", hash = "sha256:ccc"},
]

[[package]]
name = "attrs"
version = "20.3.0"
category = "main"
optional = false
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("1.1"));
        assert!(lockfile.packages[0].integrities.contains("sha256:ccc"));
    }

    #[test]
    fn keeps_git_sources_and_skips_local_ones() {
        let text = r#"[[package]]
name = "gitdep"
version = "0.2.0"
[package.source]
type = "git"
url = "https://github.com/example/gitdep"
reference = "main"
resolved_reference = "abc"

[[package]]
name = "local"
version = "0.1.0"
[package.source]
type = "directory"
url = "../local"
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.package_count(), 1);
        assert!(
            lockfile.packages[0]
                .sources
                .contains("git:https://github.com/example/gitdep#abc")
        );
    }
}
