use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_locklens"))
        .args(args)
        .env_remove("NO_COLOR")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            stdout(output),
            stderr(output)
        )
    })
}

const OLD: &str = "examples/before/package-lock.json";
const NEW: &str = "examples/after/package-lock.json";

#[test]
fn help_and_version_exit_zero() {
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help);
    assert!(text.contains("LockLens compares two lockfiles by package name"));
    assert!(text.contains("Exit codes:"));
    assert!(text.contains("--fail-on"));

    let version = run(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(stdout(&version).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn json_report_has_stable_schema_and_classified_changes() {
    let output = run(&["diff", OLD, NEW, "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stderr(&output).is_empty());

    let report = json(&output);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["tool"]["name"], "locklens");
    assert_eq!(report["old"]["path"], OLD);
    assert_eq!(report["old"]["format"], "npm");
    assert_eq!(report["old"]["packages"], 5);
    assert_eq!(report["new"]["packages"], 5);
    assert_eq!(report["summary"]["total"], 6);
    assert_eq!(report["summary"]["packages_changed"], 6);
    assert_eq!(report["summary"]["critical"], 1);
    assert_eq!(report["summary"]["error"], 0);
    assert_eq!(report["summary"]["warning"], 2);
    assert_eq!(report["summary"]["info"], 3);
    assert_eq!(report["summary"]["kinds"]["integrity"], 1);
    assert_eq!(report["summary"]["kinds"]["major"], 1);
    assert_eq!(report["summary"]["kinds"]["downgrade"], 1);

    let changes = report["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 6);

    let integrity = changes
        .iter()
        .find(|change| change["package"] == "left-pad")
        .unwrap();
    assert_eq!(integrity["kind"], "integrity_changed");
    assert_eq!(integrity["severity"], "critical");
    assert_eq!(integrity["before"], "sha512-left-pad-old");
    assert_eq!(integrity["after"], "sha512-left-pad-new");

    let major = changes
        .iter()
        .find(|change| change["package"] == "vite")
        .unwrap();
    assert_eq!(major["kind"], "major_upgrade");
    assert_eq!(major["severity"], "warning");
    assert_eq!(major["before"], "5.4.0");
    assert_eq!(major["after"], "6.0.0");

    let downgrade = changes
        .iter()
        .find(|change| change["package"] == "typescript")
        .unwrap();
    assert_eq!(downgrade["kind"], "downgrade");
    assert_eq!(downgrade["dev"], true);
}

#[test]
fn fail_on_is_report_only_by_default_and_inclusive() {
    let report_only = run(&["diff", OLD, NEW]);
    assert_eq!(report_only.status.code(), Some(0));
    assert!(stdout(&report_only).contains("Result: report only"));

    let critical = run(&["diff", OLD, NEW, "--fail-on", "critical"]);
    assert_eq!(critical.status.code(), Some(2));
    assert!(stdout(&critical).contains("--fail-on critical matched findings"));

    let error = run(&[
        "diff",
        OLD,
        NEW,
        "--ignore",
        "left-pad",
        "--fail-on",
        "error",
    ]);
    assert_eq!(error.status.code(), Some(0));
    assert!(stdout(&error).contains("Result: pass"));

    let warning = run(&[
        "diff",
        OLD,
        NEW,
        "--ignore",
        "left-pad",
        "--fail-on",
        "warning",
    ]);
    assert_eq!(warning.status.code(), Some(2));

    let any = run(&["diff", OLD, NEW, "--fail-on", "any"]);
    assert_eq!(any.status.code(), Some(2));
}

#[test]
fn sarif_output_points_at_the_relevant_lockfile() {
    let output = run(&["diff", OLD, NEW, "--sarif"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);

    assert_eq!(report["version"], "2.1.0");
    assert_eq!(report["runs"][0]["tool"]["driver"]["name"], "LockLens");
    assert_eq!(
        report["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        9
    );

    let results = report["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 6);

    let integrity = results
        .iter()
        .find(|result| result["ruleId"] == "locklens/integrity-changed")
        .unwrap();
    assert_eq!(integrity["level"], "error");
    assert_eq!(
        integrity["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        NEW
    );
    assert_eq!(
        integrity["locations"][0]["physicalLocation"]["region"]["startLine"],
        1
    );

    let removed = results
        .iter()
        .find(|result| result["ruleId"] == "locklens/removed")
        .unwrap();
    assert_eq!(removed["level"], "note");
    assert_eq!(
        removed["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        OLD
    );
    assert_eq!(removed["properties"]["package"], "chalk");
}

#[test]
fn markdown_output_is_a_paste_ready_report() {
    let output = run(&["diff", OLD, NEW, "--markdown"]);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);

    assert!(text.starts_with("# LockLens report"));
    assert!(text.contains("| Old | `examples/before/package-lock.json` | npm | 5 |"));
    assert!(text.contains("## Critical (1)"));
    assert!(text.contains("**`left-pad`**"));
    assert!(text.contains("`sha512-left-pad-old`"));
    assert!(text.contains("## Warning (2)"));
    assert!(text.contains("6 packages changed"));
}

#[test]
fn ignore_filters_packages_and_is_reported() {
    let output = run(&[
        "diff", OLD, NEW, "--json", "--ignore", "left-pad", "--ignore", "vite",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);

    assert_eq!(report["ignored"], serde_json::json!(["left-pad", "vite"]));
    assert_eq!(report["summary"]["critical"], 0);
    assert_eq!(report["summary"]["warning"], 1);
    assert!(
        report["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|change| change["package"] != "left-pad" && change["package"] != "vite")
    );
}

#[test]
fn identical_lockfiles_produce_an_empty_report() {
    let output = run(&["diff", OLD, OLD, "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);

    assert_eq!(report["summary"]["total"], 0);
    assert_eq!(report["summary"]["packages_changed"], 0);
    assert_eq!(report["summary"]["unchanged"], 5);
    assert_eq!(report["changes"], serde_json::json!([]));
}

#[test]
fn output_flags_are_mutually_exclusive() {
    let output = run(&["diff", OLD, NEW, "--json", "--sarif"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot be used with"));
    assert!(stdout(&output).is_empty());
}

#[test]
fn content_detection_handles_renamed_lockfiles() {
    let temp = TempDir::new().unwrap();
    let copied = temp.path().join("deps.json");
    fs::copy(OLD, &copied).unwrap();

    let output = run(&["diff", copied.to_str().unwrap(), NEW, "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert_eq!(report["old"]["format"], "npm");
}

#[test]
fn unreadable_and_unknown_files_fail_with_context() {
    let temp = TempDir::new().unwrap();
    let unknown = temp.path().join("deps.txt");
    fs::write(&unknown, "not a lockfile").unwrap();
    let output = run(&["diff", unknown.to_str().unwrap(), NEW]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("could not detect the lockfile format"));

    let bad = temp.path().join("package-lock.json");
    fs::write(&bad, "{ not json").unwrap();
    let output = run(&["diff", bad.to_str().unwrap(), NEW]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("parsing"));
    assert!(stderr(&output).contains("invalid JSON"));
}

#[cfg(unix)]
#[test]
fn help_through_a_closed_pipe_exits_zero() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("read -r _ || true; exec \"$0\" --help")
        .arg(env!("CARGO_BIN_EXE_locklens"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn examples_exist_and_are_files() {
    assert!(Path::new(OLD).is_file());
    assert!(Path::new(NEW).is_file());
}
