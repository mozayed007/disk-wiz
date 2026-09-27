//! End-to-end CLI tests against fixture trees.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), vec![b'x'; 2000]).unwrap();
    fs::write(root.join("src/lib.rs"), vec![b'x'; 1000]).unwrap();
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::write(root.join("target/debug/app.exe"), vec![b'x'; 8000]).unwrap();
    fs::write(root.join("readme.md"), vec![b'x'; 500]).unwrap();
    fs::write(root.join("skip.log"), vec![b'x'; 100]).unwrap();
    tmp
}

fn dw() -> Command {
    Command::cargo_bin("dw").expect("binary builds")
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn flat_lists_entries_sorted_by_size() {
    let tmp = fixture();
    let out = dw()
        .args(["-p", "--apparent-size", "--bytes", "-n", "20"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 6, "expected several rows:\n{text}");
    // The root is the largest entry.
    assert!(lines[0].ends_with(path_str(tmp.path()).as_str()));
    // app.exe (8000) outranks main.rs (2000).
    let app = lines.iter().position(|l| l.contains("app.exe")).unwrap();
    let main = lines.iter().position(|l| l.contains("main.rs")).unwrap();
    assert!(app < main, "app.exe should sort before main.rs:\n{text}");
}

#[test]
fn json_output_is_valid() {
    let tmp = fixture();
    let out = dw()
        .args(["--json", "--apparent-size", "-d", "2"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(value["size"].as_u64().unwrap() >= 11_500);
    assert_eq!(value["kind"], "dir");
    let children = value["children"].as_array().unwrap();
    let names: Vec<&str> = children
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"src"));
    assert!(names.contains(&"target"));
}

#[test]
fn tree_output_shows_branches() {
    let tmp = fixture();
    dw().args(["-p", "--format", "tree", "-d", "2"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("src").and(predicate::str::contains("main.rs")));
}

#[test]
fn ansi_output_renders_without_terminal() {
    let tmp = fixture();
    dw().args(["--format", "ansi", "--width", "60", "--height", "10"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("target"));
}

#[test]
fn tui_format_falls_back_when_piped() {
    let tmp = fixture();
    dw().args(["--format", "tui"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main.rs"));
}

#[test]
fn type_filter_keeps_only_category() {
    let tmp = fixture();
    let out = dw()
        .args(["-p", "--type", "code"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("main.rs"), "{text}");
    assert!(text.contains("lib.rs"), "{text}");
    assert!(!text.contains("app.exe"), "{text}");
}

#[test]
fn exclude_prunes_entries() {
    let tmp = fixture();
    let out = dw()
        .args(["-p", "--exclude", "*.log", "--exclude", "target"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("skip.log"), "{text}");
    assert!(!text.contains("app.exe"), "{text}");
    assert!(text.contains("main.rs"), "{text}");
}

#[test]
fn min_size_filters_rows() {
    let tmp = fixture();
    let out = dw()
        .args(["-p", "--apparent-size", "--min-size", "1K"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("skip.log"), "{text}");
    assert!(text.contains("main.rs"), "{text}");
}

#[test]
fn threshold_exit_code() {
    let tmp = fixture();
    dw().args(["-p", "--threshold", "1K"])
        .arg(tmp.path())
        .assert()
        .code(4);
    dw().args(["-p", "--threshold", "1T"])
        .arg(tmp.path())
        .assert()
        .code(0);
}

#[test]
fn missing_path_fails() {
    dw().arg("definitely/not/a/real/path")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("path not found"));
}

#[test]
fn list_palettes_and_categories() {
    dw().arg("--list-palettes")
        .assert()
        .success()
        .stdout(predicate::str::contains("viridis").and(predicate::str::contains("magma")));
    dw().arg("--list-categories")
        .assert()
        .success()
        .stdout(predicate::str::contains("code").and(predicate::str::contains("cache")));
}

#[test]
fn bad_palette_is_a_clear_error() {
    dw().args(["-p", "--palette", "nope"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown palette"));
}

#[test]
fn completions_render() {
    dw().args(["--completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dw"));
}

#[test]
fn config_file_is_applied() {
    let tmp = fixture();
    let cfg = tmp.path().join("config.toml");
    fs::write(
        &cfg,
        r#"
[scan]
hidden = false
exclude = ["*.log"]

[tui]
depth = 2

[color]
mode = "size"
palette = "magma"
"#,
    )
    .unwrap();
    let out = dw()
        .args(["-p", "--config"])
        .arg(&cfg)
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("skip.log"),
        "config exclude ignored:\n{text}"
    );
}

#[test]
fn invalid_config_reports_the_file() {
    let tmp = fixture();
    let cfg = tmp.path().join("config.toml");
    fs::write(&cfg, "[tui]\ndepth = \"deep\"\n").unwrap();
    dw().args(["-p", "--config"])
        .arg(&cfg)
        .arg(tmp.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("config"));
}
