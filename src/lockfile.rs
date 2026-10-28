use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::model::{LockFormat, Lockfile};
use crate::{cargo, npm, pnpm, poetry, uv, yarn};

/// Reads a lockfile and dispatches to the parser for its format.
pub fn load(path: &Path) -> Result<Lockfile> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading lockfile {}", path.display()))?;
    let format = detect(path, &text)?;
    let lockfile = match format {
        LockFormat::Npm => npm::parse(&text),
        LockFormat::Pnpm => pnpm::parse(&text),
        LockFormat::YarnClassic => yarn::parse_classic(&text),
        LockFormat::YarnBerry => yarn::parse_berry(&text),
        LockFormat::Cargo => cargo::parse(&text),
        LockFormat::Uv => uv::parse(&text),
        LockFormat::Poetry => poetry::parse(&text),
    }
    .with_context(|| format!("parsing {} as {}", path.display(), format.label()))?;

    if lockfile.packages.is_empty() {
        bail!(
            "{} is a {} lockfile but contains no packages",
            path.display(),
            format.label()
        );
    }
    Ok(lockfile)
}

/// Detects the format from the filename first, then from content. The content
/// fallback exists so copied or renamed lockfiles can still be reviewed.
pub fn detect(path: &Path, text: &str) -> Result<LockFormat> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    match name {
        "package-lock.json" | "npm-shrinkwrap.json" => return Ok(LockFormat::Npm),
        "pnpm-lock.yaml" => return Ok(LockFormat::Pnpm),
        "yarn.lock" => return Ok(detect_yarn(text)),
        "Cargo.lock" => return Ok(LockFormat::Cargo),
        "uv.lock" => return Ok(LockFormat::Uv),
        "poetry.lock" => return Ok(LockFormat::Poetry),
        _ => {}
    }

    if text.contains("\"lockfileVersion\"") {
        return Ok(LockFormat::Npm);
    }
    if text.contains("__metadata:") {
        return Ok(LockFormat::YarnBerry);
    }
    if text.lines().any(|line| line.trim() == "# yarn lockfile v1") {
        return Ok(LockFormat::YarnClassic);
    }
    if text.contains("lockfileVersion:") {
        return Ok(LockFormat::Pnpm);
    }
    if text.contains("[metadata]") && text.contains("lock-version") {
        return Ok(LockFormat::Poetry);
    }
    if text.contains("[[package]]") {
        if text.contains("sdist") || text.contains("wheels") {
            return Ok(LockFormat::Uv);
        }
        if text.contains("source = { registry") && text.starts_with("version = 1") {
            return Ok(LockFormat::Uv);
        }
        return Ok(LockFormat::Cargo);
    }

    bail!(
        "could not detect the lockfile format of {}; supported formats are npm, pnpm, Yarn classic, Yarn Berry, Cargo, uv, and Poetry",
        path.display()
    )
}

fn detect_yarn(text: &str) -> LockFormat {
    if text.contains("__metadata:") {
        LockFormat::YarnBerry
    } else {
        LockFormat::YarnClassic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn filenames_win_over_content() {
        assert_eq!(
            detect(&path("package-lock.json"), "{}").unwrap(),
            LockFormat::Npm
        );
        assert_eq!(
            detect(&path("pnpm-lock.yaml"), "").unwrap(),
            LockFormat::Pnpm
        );
        assert_eq!(detect(&path("Cargo.lock"), "").unwrap(), LockFormat::Cargo);
        assert_eq!(detect(&path("uv.lock"), "").unwrap(), LockFormat::Uv);
        assert_eq!(
            detect(&path("poetry.lock"), "").unwrap(),
            LockFormat::Poetry
        );
    }

    #[test]
    fn yarn_variants_are_separated_by_metadata() {
        assert_eq!(
            detect(&path("yarn.lock"), "# yarn lockfile v1\n").unwrap(),
            LockFormat::YarnClassic
        );
        assert_eq!(
            detect(&path("yarn.lock"), "__metadata:\n  version: 8\n").unwrap(),
            LockFormat::YarnBerry
        );
    }

    #[test]
    fn unknown_content_is_rejected() {
        let error = detect(&path("deps.txt"), "not a lockfile").unwrap_err();
        assert!(error.to_string().contains("could not detect"));
    }
}
