use anyhow::{Context, Result, bail};
use serde_yaml_ng::Value;

use crate::model::{LockFormat, Lockfile, Package};

/// Parses Yarn classic (`# yarn lockfile v1`) block syntax.
pub fn parse_classic(text: &str) -> Result<Lockfile> {
    let mut packages = Vec::new();
    let mut current: Option<ClassicBlock> = None;

    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if !line.starts_with(' ') {
            if let Some(block) = current.take() {
                flush_classic(block, &mut packages);
            }
            current = Some(ClassicBlock {
                header: line.trim_end_matches(':').to_owned(),
                version: None,
                resolved: None,
                integrity: None,
            });
            continue;
        }

        let Some(block) = current.as_mut() else {
            continue;
        };
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix("version ") {
            block.version = Some(unquote(value).to_owned());
        } else if let Some(value) = trimmed.strip_prefix("resolved ") {
            block.resolved = Some(unquote(value).to_owned());
        } else if let Some(value) = trimmed.strip_prefix("integrity ") {
            block.integrity = Some(unquote(value).to_owned());
        }
    }

    if let Some(block) = current.take() {
        flush_classic(block, &mut packages);
    }

    if packages.is_empty() {
        bail!("no packages found; is this a Yarn classic lockfile?");
    }
    Ok(Lockfile::new(LockFormat::YarnClassic, None, packages))
}

struct ClassicBlock {
    header: String,
    version: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
}

fn flush_classic(block: ClassicBlock, packages: &mut Vec<Package>) {
    let Some(version) = block.version else {
        return;
    };
    let Some(name) = name_from_selector(&block.header) else {
        return;
    };
    let mut package = Package::new(name, version);
    if let Some(resolved) = block.resolved {
        package.add_source(resolved);
    }
    if let Some(integrity) = block.integrity {
        package.add_integrity(integrity);
    }
    packages.push(package);
}

/// Parses Yarn Berry (`__metadata:`) YAML lockfiles.
pub fn parse_berry(text: &str) -> Result<Lockfile> {
    let value: Value = serde_yaml_ng::from_str(text).context("invalid YAML")?;
    let lockfile_version = value
        .get("__metadata")
        .and_then(|metadata| metadata.get("version"))
        .and_then(scalar_string);
    let Some(entries) = value.as_mapping() else {
        bail!("lockfile root is not a mapping");
    };

    let mut packages = Vec::new();
    for (key, entry) in entries {
        let Some(key) = key.as_str() else {
            continue;
        };
        if key.starts_with("__") {
            continue;
        }
        let Some(object) = entry.as_mapping() else {
            continue;
        };
        let version = object
            .get(Value::String("version".to_owned()))
            .and_then(scalar_string);
        let resolution = object
            .get(Value::String("resolution".to_owned()))
            .and_then(Value::as_str);
        if let Some(resolution) = resolution {
            if resolution.starts_with("workspace:") || resolution.contains("@workspace:") {
                continue;
            }
        }
        let Some(version) = version else {
            continue;
        };
        let Some(name) = resolution
            .and_then(name_from_resolution)
            .or_else(|| name_from_selector(key))
        else {
            continue;
        };

        let mut package = Package::new(name, version);
        if let Some(checksum) = object
            .get(Value::String("checksum".to_owned()))
            .and_then(Value::as_str)
        {
            package.add_integrity(checksum);
        }
        if let Some(resolution) = resolution {
            if resolution.contains("://") || resolution.starts_with("git+") {
                package.add_source(resolution);
            }
        }
        packages.push(package);
    }

    if packages.is_empty() {
        bail!("no packages found; is this a Yarn Berry lockfile?");
    }
    Ok(Lockfile::new(
        LockFormat::YarnBerry,
        lockfile_version,
        packages,
    ))
}

/// Extracts the package name from a Yarn selector such as
/// `"@scope/name@^1.0.0"` or `name@npm:^1.0.0`.
pub fn name_from_selector(selector: &str) -> Option<String> {
    let first = first_selector(selector);
    let first = unquote(first);
    let lowered = first.to_ascii_lowercase();

    for marker in ["@npm:", "@patch:", "@workspace:"] {
        if let Some(index) = lowered.find(marker) {
            return non_empty(&first[..index]);
        }
    }

    if first.starts_with('@') {
        let slash = first.find('/')?;
        let rest = &first[slash + 1..];
        let separator = rest.find('@')?;
        return non_empty(&first[..slash + 1 + separator]);
    }

    let separator = first.find('@')?;
    non_empty(&first[..separator])
}

fn name_from_resolution(resolution: &str) -> Option<String> {
    if resolution.starts_with("workspace:") || resolution.contains("@workspace:") {
        return None;
    }
    for marker in ["@npm:", "@patch:"] {
        if let Some(index) = resolution.find(marker) {
            return non_empty(&resolution[..index]);
        }
    }
    if let Some(index) = resolution.find('@') {
        return non_empty(&resolution[..index]);
    }
    None
}

fn first_selector(header: &str) -> &str {
    let mut in_quotes = false;
    for (index, character) in header.char_indices() {
        match character {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => return header[..index].trim(),
            _ => {}
        }
    }
    header.trim()
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_classic_blocks_and_scoped_names() {
        let text = r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1

"@scope/foo@^1.0.0", "@scope/foo@~1.0.1":
  version "1.0.2"
  resolved "https://registry.yarnpkg.com/@scope/foo/-/foo-1.0.2.tgz"
  integrity sha512-aaa

bar@^2.0.0:
  version "2.1.0"
  resolved "https://registry.yarnpkg.com/bar/-/bar-2.1.0.tgz"
  integrity sha512-bbb
"#;
        let lockfile = parse_classic(text).unwrap();
        assert_eq!(lockfile.package_count(), 2);
        assert_eq!(lockfile.packages[0].name, "@scope/foo");
        assert_eq!(lockfile.packages[0].version, "1.0.2");
        assert_eq!(lockfile.packages[1].name, "bar");
        assert!(
            lockfile.packages[1]
                .sources
                .contains("https://registry.yarnpkg.com/bar/-/bar-2.1.0.tgz")
        );
    }

    #[test]
    fn parses_berry_metadata_and_skips_workspaces() {
        let text = r#"__metadata:
  version: 8
  cacheKey: 10c0

"lodash@npm:^4.17.0":
  version: 4.17.21
  resolution: "lodash@npm:4.17.21"
  checksum: 10c0/aaa

"@scope/foo@npm:^2.0.0":
  version: 2.1.0
  resolution: "@scope/foo@npm:2.1.0"
  checksum: 10c0/bbb

"app@workspace:.":
  version: 0.0.0-use.local
  resolution: "app@workspace:."
"#;
        let lockfile = parse_berry(text).unwrap();
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("8"));
        assert_eq!(lockfile.package_count(), 2);
        assert_eq!(lockfile.packages[0].name, "@scope/foo");
        assert_eq!(lockfile.packages[1].name, "lodash");
        assert!(lockfile.packages[1].integrities.contains("10c0/aaa"));
    }

    #[test]
    fn extracts_names_from_selectors_and_resolutions() {
        assert_eq!(
            name_from_selector("\"@scope/foo@^1.0.0\", \"@scope/foo@~1.0.1\":").as_deref(),
            Some("@scope/foo")
        );
        assert_eq!(
            name_from_selector("lodash@npm:^4.17.0").as_deref(),
            Some("lodash")
        );
        assert_eq!(
            name_from_resolution("lodash@npm:4.17.21").as_deref(),
            Some("lodash")
        );
        assert_eq!(name_from_resolution("app@workspace:."), None);
    }
}
