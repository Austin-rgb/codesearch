//! End-to-end tests for `codesearch lint`: a fixture project with known good and bad
//! functions, the incremental index, configuration, JSON output and exit codes.

use codesearch::index::Index;
use codesearch::lint::report::{render_json, render_text};
use codesearch::lint::{lint_project, LintReport};
use codesearch::types::{FunctionSig, Visibility};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn copy_tree(source: &Path, dest: &Path) {
    for entry in walkdir::WalkDir::new(source) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(source).unwrap();
        if rel.components().any(|c| c.as_os_str() == "target") {
            continue;
        }
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A fresh copy of `tests/fixtures/lint`.
fn fixture() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    copy_tree(&manifest_dir().join("tests/fixtures/lint"), &root);
    (dir, root)
}

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// `(file, line, rule)` for every finding, in report order.
fn locations(report: &LintReport) -> Vec<(String, usize, String)> {
    report
        .findings
        .iter()
        .map(|f| (f.file.clone(), f.line, f.rule_id.clone()))
        .collect()
}

fn expect(items: &[(&str, usize, &str)]) -> Vec<(String, usize, String)> {
    items
        .iter()
        .map(|(file, line, rule)| (file.to_string(), *line, rule.to_string()))
        .collect()
}

// ------------------------------------------------------------------- fixture

#[test]
fn fixture_produces_exactly_the_expected_findings_and_score() {
    let (_guard, root) = fixture();
    let outcome = lint_project(&root, &[]).unwrap();
    let report = &outcome.report;

    assert_eq!(
        locations(report),
        expect(&[
            // accounts.rs: transfer, lookup_email, Ledger::debit
            ("src/accounts.rs", 4, "R001"),
            ("src/accounts.rs", 29, "R001"),
            ("src/accounts.rs", 52, "R001"),
            // docs.rs: no_docs, too_short, boundary_short, create_session,
            //          no_period, too_long, blank_first
            ("src/docs.rs", 3, "R002"),
            ("src/docs.rs", 6, "R002"),
            ("src/docs.rs", 12, "R002"),
            ("src/docs.rs", 15, "R002"),
            ("src/docs.rs", 18, "R002"),
            ("src/docs.rs", 21, "R002"),
            ("src/docs.rs", 33, "R002"),
        ])
    );

    let summary = &report.summary;
    assert_eq!(summary.public_functions, 19);
    assert_eq!(summary.flagged_functions, 10);
    assert_eq!(summary.total_findings, 10);
    assert_eq!(summary.findings_by_rule["R001"], 3);
    assert_eq!(summary.findings_by_rule["R002"], 7);
    assert!((summary.discoverability_score - 9.0 / 19.0).abs() < 1e-12);
}

#[test]
fn fixture_findings_carry_helpful_details() {
    let (_guard, root) = fixture();
    let report = lint_project(&root, &[]).unwrap().report;

    let transfer = &report.findings[0];
    assert_eq!(transfer.function, "transfer");
    assert_eq!(transfer.module, "accounts");
    assert!(transfer
        .message
        .contains("`from`, `to` and `amount` share the primitive type `u64`"));
    assert!(transfer
        .suggestion
        .contains("Consider a newtype such as `Amount` for parameter `amount: u64`."));

    let debit = &report.findings[2];
    assert_eq!((debit.function.as_str(), debit.line), ("debit", 52));

    let text = render_text(&report);
    assert!(text.contains("src/accounts.rs:4  R001  primitive-heavy signature in `transfer`"));
    assert!(text.contains("src/docs.rs:3  R002  public function `no_docs` has no doc comment"));
    assert!(
        text.contains("    suggestion: Add a first doc line such as: 'Does X given Y, returns Z.'")
    );
    assert!(text.contains("  Public functions:   19\n"));
    assert!(text.contains("  Flagged functions:  10\n"));
    assert!(text.contains("  Findings:           10 (R001: 3, R002: 7)\n"));
    assert!(text.contains("  Discoverability:    0.47\n"));
}

#[test]
fn opted_out_private_and_restricted_functions_are_not_reported() {
    let (_guard, root) = fixture();
    let report = lint_project(&root, &[]).unwrap().report;
    for quiet in [
        "swap_labels",
        "internal",
        "helper",
        "fmt",
        "undocumented_but_allowed",
        "both_allowed",
        "private_helper",
        "restricted",
        "boundary_ok",
        "open_store",
    ] {
        assert!(
            report.findings.iter().all(|f| f.function != quiet),
            "`{quiet}` should not be reported"
        );
    }
}

#[test]
fn rule_selection_limits_findings_and_the_summary() {
    let (_guard, root) = fixture();
    let report = lint_project(&root, &["r001".to_string()]).unwrap().report;
    assert_eq!(report.findings.len(), 3);
    assert!(report.findings.iter().all(|f| f.rule_id == "R001"));
    assert_eq!(report.summary.flagged_functions, 3);
    assert_eq!(
        report.summary.findings_by_rule.keys().collect::<Vec<_>>(),
        vec!["R001"]
    );
    assert!((report.summary.discoverability_score - 16.0 / 19.0).abs() < 1e-12);

    let err = lint_project(&root, &["R999".to_string()])
        .err()
        .expect("should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown lint rule `R999`"), "{msg}");
    assert!(msg.contains("R001, R002"), "{msg}");
}

// --------------------------------------------------------------- incremental

#[test]
fn lint_reads_the_index_and_is_incremental() {
    let (_guard, root) = fixture();

    let first = lint_project(&root, &[]).unwrap();
    assert_eq!((first.update.added, first.update.unchanged), (2, 0));

    // Second run: nothing changed, so nothing is re-read or re-parsed.
    let second = lint_project(&root, &[]).unwrap();
    assert!(!second.update.changed());
    assert_eq!(
        (
            second.update.added,
            second.update.modified,
            second.update.unchanged
        ),
        (0, 0, 2)
    );
    assert_eq!(locations(&first.report), locations(&second.report));

    // Touch one file: only that file is re-parsed and the report follows.
    let docs = fs::read_to_string(root.join("src/docs.rs")).unwrap();
    write(
        &root,
        "src/docs.rs",
        &format!("{docs}\npub fn extra() {{}}\n"),
    );
    let third = lint_project(&root, &[]).unwrap();
    assert_eq!((third.update.modified, third.update.unchanged), (1, 1));
    assert_eq!(third.report.summary.public_functions, 20);
    assert_eq!(third.report.summary.findings_by_rule["R002"], 8);
}

#[test]
fn deleted_files_disappear_from_the_report() {
    let (_guard, root) = fixture();
    lint_project(&root, &[]).unwrap();
    fs::remove_file(root.join("src/docs.rs")).unwrap();

    let outcome = lint_project(&root, &[]).unwrap();
    assert_eq!(outcome.update.removed, 1);
    assert_eq!(outcome.report.summary.public_functions, 8);
    assert_eq!(outcome.report.summary.flagged_functions, 3);
    assert!(outcome
        .report
        .findings
        .iter()
        .all(|f| f.file == "src/accounts.rs"));
}

#[test]
fn an_index_from_the_previous_schema_is_rebuilt() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "src/lib.rs", "pub fn f(a: u64, b: u64) {}\n");

    fs::create_dir_all(root.join(".codesearch")).unwrap();
    let conn = rusqlite::Connection::open(root.join(".codesearch/index.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO meta VALUES ('schema_version', '1');
         CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT);
         CREATE TABLE functions (id INTEGER PRIMARY KEY, name TEXT);",
    )
    .unwrap();
    drop(conn);

    let outcome = lint_project(root, &[]).unwrap();
    assert_eq!(outcome.update.added, 1);
    assert_eq!(outcome.report.summary.flagged_functions, 1);
}

#[test]
fn the_new_extraction_fields_round_trip_through_the_index() {
    let (_guard, root) = fixture();
    let mut index = Index::open(&root).unwrap();
    index.update(false).unwrap();

    let mut all: Vec<FunctionSig> = Vec::new();
    index
        .for_each_function(|f| {
            all.push(f);
            Ok(())
        })
        .unwrap();
    let get = |name: &str| all.iter().find(|f| f.name == name).unwrap();

    let transfer = get("transfer");
    assert_eq!(transfer.visibility, Visibility::Pub);
    assert_eq!(
        transfer.doc.as_deref(),
        Some("Transfers money between two accounts.")
    );
    assert_eq!(
        transfer.doc_first_line.as_deref(),
        Some("Transfers money between two accounts.")
    );
    assert!(transfer.lint_allow.is_empty());
    assert_eq!(transfer.trait_impl, None);

    assert_eq!(get("internal").visibility, Visibility::PubCrate);
    assert_eq!(get("helper").visibility, Visibility::Private);
    assert_eq!(get("restricted").visibility, Visibility::PubSuper);
    assert_eq!(get("no_docs").doc, None);
    assert_eq!(get("blank_first").doc_first_line.as_deref(), Some(""));

    assert_eq!(get("swap_labels").lint_allow, vec!["R001"]);
    assert_eq!(get("both_allowed").lint_allow, vec!["R001", "R002"]);
    assert_eq!(get("undocumented_but_allowed").lint_allow, vec!["R002"]);

    assert_eq!(
        get("open_store").doc.as_deref(),
        Some("Opens the store at the given location and returns a handle.\n\nFails when the location is not writable.")
    );

    let debit = get("debit");
    assert_eq!(debit.visibility, Visibility::Pub);
    assert_eq!(debit.impl_target.as_deref(), Some("Ledger"));
    assert_eq!(debit.trait_impl, None);

    let fmt = get("fmt");
    assert_eq!(fmt.visibility, Visibility::Private);
    assert_eq!(fmt.trait_impl.as_deref(), Some("std::fmt::Display"));
}

#[test]
fn the_index_gitignore_keeps_the_lint_config_committable() {
    let (_guard, root) = fixture();
    lint_project(&root, &[]).unwrap();
    let ignore = fs::read_to_string(root.join(".codesearch/.gitignore")).unwrap();
    assert!(
        ignore.contains("*") && ignore.contains("!lint.toml"),
        "{ignore}"
    );
}

// -------------------------------------------------------------------- config

#[test]
fn config_can_ignore_files_and_pick_rules() {
    let (_guard, root) = fixture();
    write(
        &root,
        ".codesearch/lint.toml",
        "ignore = [\"src/docs.rs\"]\n",
    );
    let report = lint_project(&root, &[]).unwrap().report;
    assert_eq!(report.summary.public_functions, 8);
    assert_eq!(report.findings.len(), 3);

    write(
        &root,
        ".codesearch/lint.toml",
        "enabled_rules = [\"R002\"]\n",
    );
    let report = lint_project(&root, &[]).unwrap().report;
    assert_eq!(report.findings.len(), 7);
    assert!(report.findings.iter().all(|f| f.rule_id == "R002"));

    // `--rule` beats the config file.
    let report = lint_project(&root, &["R001".to_string()]).unwrap().report;
    assert_eq!(report.findings.len(), 3);
}

#[test]
fn config_can_change_the_minimum_intent_length() {
    let (_guard, root) = fixture();
    // "Reads a token." has 14 characters, so it passes with a minimum of 14.
    write(&root, ".codesearch/lint.toml", "min_intent_len = 14\n");
    let report = lint_project(&root, &[]).unwrap().report;
    let r002_lines: Vec<usize> = report
        .findings
        .iter()
        .filter(|f| f.rule_id == "R002")
        .map(|f| f.line)
        .collect();
    assert_eq!(r002_lines, vec![3, 6, 15, 18, 21, 33]);
}

#[test]
fn config_can_replace_the_domain_words() {
    let (_guard, root) = fixture();
    write(&root, ".codesearch/lint.toml", "domain_words = [\"sku\"]\n");
    let report = lint_project(&root, &["R001".to_string()]).unwrap().report;
    // Only `transfer` still trips clause (a) / (c); `lookup_email` and `debit` needed (b).
    assert_eq!(
        locations(&report),
        expect(&[("src/accounts.rs", 4, "R001")])
    );
}

#[test]
fn a_broken_config_is_an_error() {
    let (_guard, root) = fixture();
    write(
        &root,
        ".codesearch/lint.toml",
        "min_intent_len = \"long\"\n",
    );
    let err = lint_project(&root, &[]).err().expect("should fail");
    assert!(
        format!("{err:#}").contains("invalid lint config"),
        "{err:#}"
    );
}

// ---------------------------------------------------------------- sample project

#[test]
fn sample_project_lint_matches_the_documented_output() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    copy_tree(&manifest_dir().join("examples/sample-project"), root);

    let report = lint_project(root, &[]).unwrap().report;
    assert_eq!(
        locations(&report),
        expect(&[
            ("src/auth/service.rs", 4, "R002"),
            ("src/auth/service.rs", 13, "R002"),
            ("src/auth/service.rs", 26, "R002"),
            ("src/billing.rs", 2, "R001"),
            ("src/billing.rs", 7, "R001"),
            ("src/billing.rs", 12, "R001"),
            ("src/config.rs", 3, "R002"),
            ("src/config.rs", 9, "R002"),
            ("src/config.rs", 13, "R002"),
            ("src/users.rs", 3, "R002"),
            ("src/users.rs", 26, "R002"),
            ("src/users.rs", 39, "R002"),
            ("src/users.rs", 43, "R002"),
            ("src/users.rs", 47, "R002"),
        ])
    );
    assert_eq!(report.summary.public_functions, 21);
    assert_eq!(report.summary.flagged_functions, 14);
    assert!((report.summary.discoverability_score - 7.0 / 21.0).abs() < 1e-12);
}

// ------------------------------------------------------------------ the binary

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codesearch"))
        .args(args)
        .output()
        .expect("failed to run the codesearch binary")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn json_output_parses_and_has_the_documented_fields() {
    let (_guard, root) = fixture();
    let out = run(&["lint", root.to_str().unwrap(), "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    let findings = json["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 10);
    for finding in findings {
        for key in [
            "rule_id",
            "severity",
            "file",
            "line",
            "module",
            "function",
            "message",
            "suggestion",
        ] {
            assert!(
                !finding[key].is_null(),
                "finding is missing `{key}`: {finding}"
            );
        }
        assert_eq!(finding["severity"], "warn");
        assert!(finding["line"].is_u64());
    }
    assert_eq!(findings[0]["rule_id"], "R001");
    assert_eq!(findings[0]["file"], "src/accounts.rs");
    assert_eq!(findings[0]["function"], "transfer");

    let summary = &json["summary"];
    assert_eq!(summary["public_functions"], 19);
    assert_eq!(summary["flagged_functions"], 10);
    assert_eq!(summary["findings_by_rule"]["R001"], 3);
    assert_eq!(summary["findings_by_rule"]["R002"], 7);
    let score = summary["discoverability_score"].as_f64().unwrap();
    assert!((score - 9.0 / 19.0).abs() < 1e-12);
}

#[test]
fn json_output_matches_the_library_renderer() {
    let (_guard, root) = fixture();
    let report = lint_project(&root, &[]).unwrap().report;
    let out = run(&["lint", root.to_str().unwrap(), "--format", "json"]);
    let from_cli: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let from_lib: serde_json::Value = serde_json::from_str(&render_json(&report).unwrap()).unwrap();
    assert_eq!(from_cli, from_lib);
}

#[test]
fn text_output_and_the_project_flag() {
    let (_guard, root) = fixture();
    let out = run(&["--project", root.to_str().unwrap(), "lint"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.contains("src/accounts.rs:4  R001  "), "{text}");
    assert!(text.contains("Summary"), "{text}");
}

#[test]
fn exit_status_follows_fail_on() {
    let (_guard, root) = fixture();
    let path = root.to_str().unwrap();

    assert_eq!(run(&["lint", path]).status.code(), Some(0));
    assert_eq!(
        run(&["lint", path, "--fail-on", "never"]).status.code(),
        Some(0)
    );
    assert_eq!(
        run(&["lint", path, "--fail-on", "warn"]).status.code(),
        Some(1)
    );

    // Findings still print before the non-zero exit.
    assert!(stdout(&run(&["lint", path, "--fail-on", "warn"])).contains("R001"));

    // No findings: exit 0 even with --fail-on warn.
    let clean = TempDir::new().unwrap();
    write(
        clean.path(),
        "src/lib.rs",
        "/// Adds two numbers together safely.\npub fn add(a: Number, b: Number) -> Number { todo!() }\n",
    );
    let out = run(&["lint", clean.path().to_str().unwrap(), "--fail-on", "warn"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("No discoverability findings."));
    assert!(stdout(&out).contains("Discoverability:    1.00"));
}

#[test]
fn errors_exit_with_status_two() {
    let (_guard, root) = fixture();
    let path = root.to_str().unwrap();

    let out = run(&["lint", path, "--rule", "R999"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("unknown lint rule `R999`"),
        "{}",
        stderr(&out)
    );

    let missing = root.join("does-not-exist");
    let out = run(&["lint", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("error:"));

    let out = run(&["lint", path, "extra-argument"]);
    assert_eq!(out.status.code(), Some(2));

    // clap rejects bad enum values with its own usage error (also status 2).
    assert_eq!(
        run(&["lint", path, "--format", "yaml"]).status.code(),
        Some(2)
    );
    assert_eq!(
        run(&["lint", path, "--fail-on", "info"]).status.code(),
        Some(2)
    );
}

#[test]
fn rule_flag_accepts_a_comma_separated_list() {
    let (_guard, root) = fixture();
    let out = run(&[
        "lint",
        root.to_str().unwrap(),
        "--rule",
        "R001,R002",
        "--format",
        "json",
    ]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(json["findings"].as_array().unwrap().len(), 10);

    let out = run(&[
        "lint",
        root.to_str().unwrap(),
        "--rule",
        "R002",
        "--format",
        "json",
    ]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(json["findings"].as_array().unwrap().len(), 7);
}

#[test]
fn searching_still_works_alongside_lint() {
    let (_guard, root) = fixture();
    lint_project(&root, &[]).unwrap();
    let out = run(&["-p", root.to_str().unwrap(), "(u64, u64, u64) -> bool"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("1. transfer"), "{}", stdout(&out));
}
