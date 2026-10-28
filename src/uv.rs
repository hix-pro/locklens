use anyhow::{Context, Result, bail};
use toml::Value;

use crate::model::{LockFormat, Lockfile, Package};

/// Parses `uv.lock`. Local editable, virtual, directory, and path packages are
/// skipped; their versions track the workspace, not a dependency update.
pub fn parse(text: &str) -> Result<Lockfile> {
    let value: Value = toml::from_str(text).context("invalid TOML")?;
    let lockfile_version = value
        .get("version")
        .and_then(Value::as_integer)
        .map(|version| version.to_string());
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
        if let Some(sdist) = entry.get("sdist") {
            if let Some(hash) = sdist.get("hash").and_then(Value::as_str) {
                package.add_integrity(hash);
            }
        }
        if let Some(wheels) = entry.get("wheels").and_then(Value::as_array) {
            for wheel in wheels {
                if let Some(hash) = wheel.get("hash").and_then(Value::as_str) {
                    package.add_integrity(hash);
                }
            }
        }
        packages.push(package);
    }

    if packages.is_empty() {
        bail!("no registry or git packages found");
    }
    Ok(Lockfile::new(LockFormat::Uv, lockfile_version, packages))
}

fn is_local(source: Option<&Value>) -> bool {
    let Some(source) = source.and_then(Value::as_table) else {
        return false;
    };
    ["virtual", "editable", "directory", "path"]
        .iter()
        .any(|key| source.contains_key(*key))
}

fn source_label(source: &Value) -> Option<String> {
    let table = source.as_table()?;
    if let Some(registry) = table.get("registry").and_then(Value::as_str) {
        return Some(format!("registry:{registry}"));
    }
    if let Some(git) = table.get("git").and_then(Value::as_str) {
        return Some(git.to_owned());
    }
    if let Some(url) = table.get("url").and_then(Value::as_str) {
        return Some(url.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_packages_and_skips_editable_ones() {
        let text = r#"version = 1
revision = 3

[[package]]
name = "anyio"
version = "4.6.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://files.pythonhosted.org/anyio.tar.gz", hash = "sha256:aaa", size = 1 }
wheels = [
  { url = "https://files.pythonhosted.org/anyio.whl", hash = "sha256:bbb" },
]

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "." }
"#;
        let lockfile = parse(text).unwrap();
        assert_eq!(lockfile.lockfile_version.as_deref(), Some("1"));
        assert_eq!(lockfile.package_count(), 1);
        assert_eq!(lockfile.packages[0].name, "anyio");
        assert_eq!(lockfile.packages[0].integrities.len(), 2);
        assert!(
            lockfile.packages[0]
                .sources
                .contains("registry:https://pypi.org/simple")
        );
    }

    #[test]
    fn keeps_git_sources() {
        let text = r#"version = 1

[[package]]
name = "gitdep"
version = "0.2.0"
source = { git = "https://github.com/example/gitdep?rev=abc#abc" }
"#;
        let lockfile = parse(text).unwrap();
        assert!(
            lockfile.packages[0]
                .sources
                .contains("https://github.com/example/gitdep?rev=abc#abc")
        );
    }
}
