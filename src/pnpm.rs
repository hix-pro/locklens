use anyhow::{Context, Result, bail};
use serde_yaml_ng::{Mapping, Value};

use crate::model::{LockFormat, Lockfile, Package};

/// Parses `pnpm-lock.yaml` across the v5, v6, and v9 shapes. Only the package
/// identities and resolution metadata are used; the dependency graph is not
/// needed to answer "what changed".
pub fn parse(text: &str) -> Result<Lockfile> {
    let value: Value = serde_yaml_ng::from_str(text).context("invalid YAML")?;
    let lockfile_version = value.get("lockfileVersion").and_then(scalar_string);

    let mut packages = Vec::new();
    let entries = value
        .get("packages")
        .and_then(Value::as_mapping)
        .or_else(|| value.get("snapshots").and_then(Value::as_mapping));
    let Some(entries) = entries else {
        bail!("no packages or snapshots mapping found");
    };

    for (key, entry) in entries {
        let Some(key) = key.as_str() else {
            continue;
        };
        let Some((name, version)) = split_package_key(key) else {
            continue;
        };
        let mut package = Package::new(name, version);
        let Some(object) = entry.as_mapping() else {
            packages.push(package);
            continue;
        };

        if let Some(resolution) = object.get(Value::String("resolution".to_owned())) {
            apply_resolution(&mut package, resolution);
        }
        package = package
            .dev(mapping_bool(object, "dev"))
            .optional(mapping_bool(object, "optional"));
        packages.push(package);
    }

    Ok(Lockfile::new(LockFormat::Pnpm, lockfile_version, packages))
}

fn apply_resolution(package: &mut Package, resolution: &Value) {
    if let Some(text) = resolution.as_str() {
        package.add_source(text);
        return;
    }
    let Some(object) = resolution.as_mapping() else {
        return;
    };
    if let Some(integrity) = mapping_str(object, "integrity") {
        package.add_integrity(integrity);
    }
    if let Some(tarball) = mapping_str(object, "tarball") {
        package.add_source(tarball);
    }
    if let Some(repo) = mapping_str(object, "repo") {
        package.add_source(repo);
    }
}

fn split_package_key(key: &str) -> Option<(String, String)> {
    let key = key.trim().trim_start_matches('/');
    if key.is_empty() {
        return None;
    }

    // pnpm v9 can append the peer set: `react-dom@18.2.0(react@18.2.0)`.
    let key = key.split('(').next().unwrap_or(key);

    if let Some((name, version)) = split_at_version(key) {
        return Some((name, version));
    }

    // pnpm v5 used `/name/1.2.3` and `/@scope/name/1.2.3`.
    let (name, version) = key.rsplit_once('/')?;
    if name.is_empty() || version.is_empty() || version.starts_with('@') {
        return None;
    }
    Some((name.trim_start_matches('/').to_owned(), version.to_owned()))
}

fn split_at_version(key: &str) -> Option<(String, String)> {
    let separator = key.rfind('@')?;
    if separator == 0 {
        return None;
    }
    let name = &key[..separator];
    let version = &key[separator + 1..];
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_owned(), version.to_owned()))
}

fn mapping_bool(object: &Mapping, key: &str) -> bool {
    object
        .get(Value::String(key.to_owned()))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn mapping_str<'a>(object: &'a Mapping, key: &str) -> Option<&'a str> {
    object
        .get(Value::String(key.to_owned()))
        .and_then(Value::as_str)
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
    fn parses_v9_package_keys_with_peer_sets() {
        let text = r#"
lockfileVersion: '9.0'
packages:
  lodash@4.17.21:
    resolution: {integrity: sha512-aaa}
  '@scope/foo@1.2.3':
    resolution: {integrity: sha512-bbb}
  react-dom@18.2.0(react@18.2.0):
    resolution: {integrity: sha512-ccc}
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("9.0"));
        assert_eq!(lockfile.package_count(), 3);
        let names: Vec<_> = lockfile
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package.version.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("@scope/foo", "1.2.3"),
                ("lodash", "4.17.21"),
                ("react-dom", "18.2.0"),
            ]
        );
    }

    #[test]
    fn parses_v6_and_v5_keys() {
        let text = r#"
lockfileVersion: 5.4
packages:
  /foo@1.2.3:
    resolution: {integrity: sha512-aaa}
    dev: true
  /@scope/bar@2.0.0:
    resolution: {integrity: sha512-bbb}
    optional: true
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.package_count(), 2);
        assert_eq!(lockfile.packages[0].name, "@scope/bar");
        assert!(lockfile.packages[0].optional);
        assert!(lockfile.packages[1].dev);
    }

    #[test]
    fn parses_v5_slash_keys() {
        let text = r#"
lockfileVersion: 5.3
packages:
  /foo/1.2.3:
    resolution: {integrity: sha512-aaa}
  /@scope/bar/2.0.0:
    resolution: {integrity: sha512-bbb}
"#;
        let lockfile = parse(text).unwrap();
        let names: Vec<_> = lockfile
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package.version.as_str()))
            .collect();
        assert_eq!(names, [("@scope/bar", "2.0.0"), ("foo", "1.2.3")]);
    }

    #[test]
    fn falls_back_to_snapshots_and_rejects_empty_files() {
        let text = r#"
lockfileVersion: '9.0'
snapshots:
  foo@1.0.0: {}
"#;
        assert_eq!(parse(text).unwrap().package_count(), 1);
        assert!(parse("lockfileVersion: '9.0'\n").is_err());
    }
}
