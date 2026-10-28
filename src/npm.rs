use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::model::{LockFormat, Lockfile, Package};

/// Parses `package-lock.json` (lockfile versions 2 and 3) and the legacy
/// lockfile version 1 nested `dependencies` shape.
pub fn parse(text: &str) -> Result<Lockfile> {
    let value: Value = serde_json::from_str(text).context("invalid JSON")?;
    let lockfile_version = value.get("lockfileVersion").and_then(scalar_string);
    let mut packages = Vec::new();

    if let Some(entries) = value.get("packages").and_then(Value::as_object) {
        for (path, entry) in entries {
            if path.is_empty() {
                continue;
            }
            let Some(object) = entry.as_object() else {
                continue;
            };
            if object.get("link").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let Some(name) = package_name(path) else {
                continue;
            };
            let Some(version) = object.get("version").and_then(Value::as_str) else {
                continue;
            };
            packages.push(package_from_entry(name, version, object));
        }
    } else if let Some(dependencies) = value.get("dependencies").and_then(Value::as_object) {
        collect_v1(dependencies, &mut packages);
    } else {
        bail!("no packages or dependencies object found");
    }

    Ok(Lockfile::new(LockFormat::Npm, lockfile_version, packages))
}

fn package_from_entry(name: String, version: &str, entry: &Map<String, Value>) -> Package {
    let mut package = Package::new(name, version);
    if let Some(resolved) = entry.get("resolved").and_then(Value::as_str) {
        package = package.with_source(resolved);
    }
    if let Some(integrity) = entry.get("integrity").and_then(Value::as_str) {
        package = package.with_integrity(integrity);
    }
    let dev = entry.get("dev").and_then(Value::as_bool).unwrap_or(false)
        || entry
            .get("devOptional")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let optional = entry
        .get("optional")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    package.dev(dev).optional(optional)
}

/// Lockfile version 1 nests dependencies by name instead of using paths.
fn collect_v1(dependencies: &Map<String, Value>, packages: &mut Vec<Package>) {
    for (name, entry) in dependencies {
        let Some(object) = entry.as_object() else {
            continue;
        };
        if let Some(version) = object.get("version").and_then(Value::as_str) {
            packages.push(package_from_entry(name.clone(), version, object));
        }
        if let Some(nested) = object.get("dependencies").and_then(Value::as_object) {
            collect_v1(nested, packages);
        }
    }
}

/// Extracts `foo` from `node_modules/foo` and `@scope/foo` from
/// `node_modules/@scope/foo`, including nested `node_modules` paths.
fn package_name(path: &str) -> Option<String> {
    let marker = "node_modules/";
    let start = path.rfind(marker)? + marker.len();
    let rest = &path[start..];
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }

    let mut parts = rest.split('/');
    let first = parts.next()?;
    if first.is_empty() {
        return None;
    }
    if first.starts_with('@') {
        let second = parts.next()?;
        if second.is_empty() {
            return None;
        }
        return Some(format!("{first}/{second}"));
    }
    Some(first.to_owned())
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
    fn parses_v3_packages_and_skips_links() {
        let text = r#"{
          "name": "fixture",
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "fixture", "version": "1.0.0" },
            "node_modules/foo": {
              "version": "1.2.3",
              "resolved": "https://registry.npmjs.org/foo/-/foo-1.2.3.tgz",
              "integrity": "sha512-aaa",
              "dev": true
            },
            "node_modules/foo/node_modules/@scope/bar": {
              "version": "2.0.0",
              "resolved": "https://registry.npmjs.org/@scope/bar/-/bar-2.0.0.tgz",
              "integrity": "sha512-bbb",
              "optional": true
            },
            "node_modules/local": { "link": true, "resolved": "../local" }
          }
        }"#;

        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.format, LockFormat::Npm);
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("3"));
        assert_eq!(lockfile.package_count(), 2);
        let foo = lockfile
            .packages
            .iter()
            .find(|package| package.name == "foo")
            .unwrap();
        assert!(foo.dev);
        let scoped = lockfile
            .packages
            .iter()
            .find(|package| package.name == "@scope/bar")
            .unwrap();
        assert!(scoped.optional);
    }

    #[test]
    fn parses_v1_nested_dependencies() {
        let text = r#"{
          "lockfileVersion": 1,
          "dependencies": {
            "foo": {
              "version": "1.0.0",
              "integrity": "sha512-aaa",
              "dependencies": {
                "bar": { "version": "2.0.0", "integrity": "sha512-bbb" }
              }
            }
          }
        }"#;

        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.package_count(), 2);
        let names: Vec<_> = lockfile
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        assert_eq!(names, ["bar", "foo"]);
    }

    #[test]
    fn rejects_a_lockfile_without_packages() {
        let error = parse(r#"{"lockfileVersion": 3}"#).unwrap_err();
        assert!(error.to_string().contains("no packages"));
    }

    #[test]
    fn package_names_handle_scopes_and_nesting() {
        assert_eq!(package_name("node_modules/foo").as_deref(), Some("foo"));
        assert_eq!(
            package_name("node_modules/@scope/foo").as_deref(),
            Some("@scope/foo")
        );
        assert_eq!(
            package_name("node_modules/a/node_modules/@scope/b").as_deref(),
            Some("@scope/b")
        );
        assert_eq!(package_name("packages/app"), None);
    }
}
