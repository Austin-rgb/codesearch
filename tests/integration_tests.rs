//! End-to-end tests: real files on disk, a real SQLite index, and the real binary.

use codesearch::index::Index;
use codesearch::parser::parse_query;
use codesearch::search::{format_results, search_project, SearchOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Searches with default options and returns `file::name` for every hit.
fn find(root: &Path, query: &str) -> Vec<String> {
    find_with(root, query, SearchOptions::default())
}

fn find_with(root: &Path, query: &str, opts: SearchOptions) -> Vec<String> {
    let query = parse_query(query).unwrap();
    let outcome = search_project(root, &query, &opts).unwrap();
    outcome
        .results
        .iter()
        .map(|f| format!("{}::{}", f.file, f.name))
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

// ------------------------------------------------------------ indexing

#[test]
fn first_search_creates_the_index_automatically() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/lib.rs", "fn a(x: A) -> B { todo!() }");

    assert!(!dir.path().join(".codesearch").exists());
    assert_eq!(find(dir.path(), "A -> B"), vec!["src/lib.rs::a"]);
    assert!(dir.path().join(".codesearch/index.db").exists());
}

#[test]
fn initial_then_incremental_indexing() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn a(x: A) -> B { todo!() }");
    write(
        dir.path(),
        "src/b.rs",
        "fn b(x: A) -> B { todo!() }\nfn c() {}",
    );

    let mut index = Index::open(dir.path()).unwrap();

    let first = index.update(false).unwrap();
    assert_eq!(
        (first.added, first.modified, first.removed, first.unchanged),
        (2, 0, 0, 0)
    );
    assert_eq!(index.stats().unwrap().functions, 3);

    let second = index.update(false).unwrap();
    assert_eq!(
        (
            second.added,
            second.modified,
            second.removed,
            second.unchanged
        ),
        (0, 0, 0, 2)
    );
    assert!(!second.changed());
}

#[test]
fn modified_files_are_reparsed_and_old_entries_replaced() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn old_name(x: A) -> B { todo!() }");
    write(dir.path(), "src/b.rs", "fn stays(x: A) -> B { todo!() }");

    let mut index = Index::open(dir.path()).unwrap();
    index.update(false).unwrap();

    write(
        dir.path(),
        "src/a.rs",
        "fn new_name(x: A, y: A) -> B { todo!() }\nfn extra() {}",
    );
    let report = index.update(false).unwrap();
    assert_eq!((report.modified, report.unchanged, report.added), (1, 1, 0));

    assert_eq!(find(dir.path(), "A -> B"), vec!["src/b.rs::stays"]);
    assert_eq!(find(dir.path(), "(A, A) -> B"), vec!["src/a.rs::new_name"]);
    assert_eq!(index.stats().unwrap().functions, 3);
}

#[test]
fn rewriting_identical_content_counts_as_unchanged() {
    let dir = TempDir::new().unwrap();
    let source = "fn a(x: A) -> B { todo!() }";
    write(dir.path(), "src/a.rs", source);

    let mut index = Index::open(dir.path()).unwrap();
    index.update(false).unwrap();

    write(dir.path(), "src/a.rs", source); // new mtime, same hash
    let report = index.update(false).unwrap();
    assert_eq!((report.modified, report.unchanged), (0, 1));

    // `force` re-hashes everything but still does not re-parse identical content
    let forced = index.update(true).unwrap();
    assert_eq!((forced.modified, forced.unchanged), (0, 1));
}

#[test]
fn deleted_files_are_removed_from_the_index() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn a(x: A) -> B { todo!() }");
    write(dir.path(), "src/b.rs", "fn b(x: A) -> B { todo!() }");

    let mut index = Index::open(dir.path()).unwrap();
    index.update(false).unwrap();
    assert_eq!(find(dir.path(), "A -> B").len(), 2);

    fs::remove_file(dir.path().join("src/a.rs")).unwrap();
    let report = index.update(false).unwrap();
    assert_eq!((report.removed, report.unchanged), (1, 1));

    let stats = index.stats().unwrap();
    assert_eq!((stats.files, stats.functions), (1, 1));
    assert_eq!(find(dir.path(), "A -> B"), vec!["src/b.rs::b"]);
}

#[test]
fn new_files_are_picked_up_by_the_next_search() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn a(x: A) -> B { todo!() }");
    assert_eq!(find(dir.path(), "A -> B").len(), 1);

    write(
        dir.path(),
        "src/deep/nested/b.rs",
        "fn b(x: A) -> B { todo!() }",
    );
    assert_eq!(
        find(dir.path(), "A -> B"),
        vec!["src/a.rs::a", "src/deep/nested/b.rs::b"]
    );
}

#[test]
fn no_update_searches_the_index_as_it_was() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn a(x: A) -> B { todo!() }");
    assert_eq!(find(dir.path(), "A -> B").len(), 1);

    write(dir.path(), "src/b.rs", "fn b(x: A) -> B { todo!() }");
    let stale = SearchOptions {
        update_index: false,
        ..SearchOptions::default()
    };
    assert_eq!(find_with(dir.path(), "A -> B", stale).len(), 1);
    assert_eq!(find(dir.path(), "A -> B").len(), 2);
}

#[test]
fn unparsable_files_are_skipped_and_retried_when_they_change() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/good.rs", "fn good(x: A) -> B { todo!() }");
    write(dir.path(), "src/bad.rs", "fn broken( {");

    let mut index = Index::open(dir.path()).unwrap();
    let report = index.update(false).unwrap();
    assert_eq!(report.parse_failures, 1);
    assert_eq!(index.stats().unwrap().parse_failures, 1);
    assert_eq!(find(dir.path(), "A -> B"), vec!["src/good.rs::good"]);

    write(
        dir.path(),
        "src/bad.rs",
        "fn fixed(x: A) -> B { todo!() }\n",
    );
    let report = index.update(false).unwrap();
    assert_eq!((report.modified, report.parse_failures), (1, 0));
    assert_eq!(index.stats().unwrap().parse_failures, 0);
    assert_eq!(find(dir.path(), "A -> B").len(), 2);
}

#[test]
fn only_rust_files_outside_ignored_directories_are_indexed() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/a.rs", "fn a(x: A) -> B { todo!() }");
    write(dir.path(), "notes.txt", "fn text(x: A) -> B");
    write(dir.path(), "script.py", "def py(x): pass");
    write(
        dir.path(),
        "target/debug/gen.rs",
        "fn built(x: A) -> B { todo!() }",
    );
    write(
        dir.path(),
        ".git/hooks/x.rs",
        "fn hook(x: A) -> B { todo!() }",
    );
    write(
        dir.path(),
        "node_modules/pkg/x.rs",
        "fn vendored(x: A) -> B { todo!() }",
    );

    assert_eq!(find(dir.path(), "A -> B"), vec!["src/a.rs::a"]);
    assert_eq!(Index::open(dir.path()).unwrap().stats().unwrap().files, 1);
}

#[test]
fn the_index_is_project_local() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    write(a.path(), "src/lib.rs", "fn in_a(x: A) -> B { todo!() }");
    write(b.path(), "src/lib.rs", "fn in_b(x: A) -> B { todo!() }");

    assert_eq!(find(a.path(), "A -> B"), vec!["src/lib.rs::in_a"]);
    assert_eq!(find(b.path(), "A -> B"), vec!["src/lib.rs::in_b"]);
    assert!(a.path().join(".codesearch/index.db").exists());
    assert!(b.path().join(".codesearch/index.db").exists());
}

#[test]
fn opening_a_missing_project_is_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nope");
    let err = Index::open(&missing).err().expect("should fail");
    assert!(err.to_string().contains("project directory not found"));
}

// ------------------------------------------------------------- sample project

fn sample_project() -> (TempDir, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sample-project");
    let dir = TempDir::new().unwrap();
    let dest = dir.path().to_path_buf();

    for entry in walkdir::WalkDir::new(&source) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(&source).unwrap();
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
    (dir, dest)
}

#[test]
fn sample_project_queries_from_the_spec() {
    let (_guard, root) = sample_project();

    assert_eq!(
        sorted(find(&root, "(Username, Password) -> Session")),
        vec![
            "src/auth/login.rs::login",
            "src/auth/service.rs::authenticate"
        ]
    );
    assert_eq!(
        find(&root, "(Username, Password) -> Result<Session, AuthError>"),
        vec!["src/auth/login.rs::try_login"]
    );
    assert_eq!(
        find(&root, "UserId -> Option<UserDetails>"),
        vec!["src/users.rs::get_user"]
    );
    assert_eq!(
        find(&root, "UserId -> Result<UserDetails, AuthError>"),
        vec!["src/users.rs::find_user"]
    );
    assert_eq!(
        sorted(find(&root, "() -> Config")),
        vec![
            "src/config.rs::default_config",
            "src/config.rs::load_config"
        ]
    );
    assert_eq!(find(&root, "UserId -> ()"), vec!["src/config.rs::log_out"]);
    assert_eq!(
        find(&root, "(UserId, TenantId) -> (User, Metadata)"),
        vec!["src/users.rs::user_and_tenant"]
    );
    assert_eq!(
        sorted(find(&root, "UserId -> UserDetails")),
        vec!["src/users.rs::details_for"]
    );
}

#[test]
fn sample_project_wildcards() {
    let (_guard, root) = sample_project();

    assert_eq!(
        sorted(find(&root, "(Username, *) -> Session")),
        vec![
            "src/auth/login.rs::login",
            "src/auth/login.rs::login_with_label",
            "src/auth/service.rs::authenticate",
        ]
    );
    assert_eq!(
        sorted(find(&root, "(Username, Password) -> *")),
        vec![
            "src/auth/login.rs::login",
            "src/auth/login.rs::try_login",
            "src/auth/service.rs::authenticate",
        ]
    );
}

#[test]
fn sample_project_parameter_order() {
    let (_guard, root) = sample_project();
    assert_eq!(
        find(&root, "(Password, Username) -> Session"),
        vec!["src/auth/login.rs::login_swapped"]
    );
}

#[test]
fn sample_project_methods_need_ignore_self() {
    let (_guard, root) = sample_project();

    let hidden = SearchOptions {
        ignore_self: true,
        ..SearchOptions::default()
    };
    assert_eq!(
        sorted(find_with(&root, "(Username, Password) -> Session", hidden)),
        vec![
            "src/auth/login.rs::login",
            "src/auth/service.rs::authenticate",
            "src/auth/service.rs::session_for",
        ]
    );
    assert_eq!(
        sorted(find_with(&root, "UserId -> Option<UserDetails>", hidden)),
        vec!["src/users.rs::get_user", "src/users.rs::lookup"]
    );

    // explicit receiver in the query works without the flag
    assert_eq!(
        find(&root, "(&Self, UserId) -> Option<UserDetails>"),
        vec!["src/users.rs::lookup"]
    );
}

#[test]
fn sample_project_results_carry_location_details() {
    let (_guard, root) = sample_project();
    let query = parse_query("(Username, Password) -> Session").unwrap();
    let outcome = search_project(&root, &query, &SearchOptions::default()).unwrap();

    let login = outcome.results.iter().find(|f| f.name == "login").unwrap();
    assert_eq!(login.file, "src/auth/login.rs");
    assert_eq!(login.line, 4);
    assert_eq!(login.module, "auth::login");

    let text = format_results(&outcome.results);
    assert!(text.starts_with("Found 2 matching functions:"));
    assert!(text.contains("   Signature: (Username, Password) -> Session"));
    assert!(text.contains("   File: src/auth/login.rs"));
    assert!(text.contains("   Line: 4"));
    assert!(text.trim_end().ends_with("2 matches found."));
}

#[test]
fn no_matches_prints_the_documented_message() {
    let (_guard, root) = sample_project();
    let query = parse_query("(Nothing, Matches) -> Here").unwrap();
    let outcome = search_project(&root, &query, &SearchOptions::default()).unwrap();
    assert!(outcome.results.is_empty());
    assert_eq!(
        format_results(&outcome.results),
        "No matching functions found.\n"
    );
}

// ---------------------------------------------------------------- the binary

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
fn cli_search_with_project_flag() {
    let (_guard, root) = sample_project();
    let root = root.to_str().unwrap();

    let out = run(&["--project", root, "(Username, Password) -> Session"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Found 2 matching functions:"), "{text}");
    assert!(
        text.contains("1. authenticate") || text.contains("1. login"),
        "{text}"
    );
    assert!(text.contains("2 matches found."), "{text}");
}

#[test]
fn cli_reports_no_matches() {
    let (_guard, root) = sample_project();
    let out = run(&["-p", root.to_str().unwrap(), "() -> Nothing"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("No matching functions found."));
}

#[test]
fn cli_index_then_stats() {
    let (_guard, root) = sample_project();
    let root = root.to_str().unwrap();

    let out = run(&["index", root]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("added"), "{}", stdout(&out));

    let again = run(&["index", root]);
    assert!(
        stdout(&again).contains("0 added, 0 modified, 0 removed"),
        "{}",
        stdout(&again)
    );

    let stats = run(&["--project", root, "stats"]);
    assert!(stats.status.success(), "{}", stderr(&stats));
    let text = stdout(&stats);
    assert!(text.contains("Files:"), "{text}");
    assert!(text.contains("Functions:"), "{text}");
    assert!(text.contains("methods"), "{text}");
}

#[test]
fn cli_ignore_self_flag() {
    let (_guard, root) = sample_project();
    let root = root.to_str().unwrap();

    let without = stdout(&run(&["-p", root, "UserId -> Option<UserDetails>"]));
    assert!(without.contains("1 match found."), "{without}");

    let with = stdout(&run(&[
        "-p",
        root,
        "--ignore-self",
        "UserId -> Option<UserDetails>",
    ]));
    assert!(with.contains("2 matches found."), "{with}");
}

#[test]
fn cli_rejects_malformed_queries_with_a_useful_message() {
    let (_guard, root) = sample_project();
    let root = root.to_str().unwrap();

    let out = run(&["-p", root, "Username, Password -> Session"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("invalid query"), "{err}");
    assert!(err.contains("parentheses"), "{err}");
    assert!(err.contains('^'), "{err}");

    // a malformed query must not create an index as a side effect
    let fresh = TempDir::new().unwrap();
    let out = run(&["-p", fresh.path().to_str().unwrap(), "(A, B"]);
    assert!(!out.status.success());
    assert!(!fresh.path().join(".codesearch").exists());
}

#[test]
fn cli_without_a_query_explains_what_to_do() {
    let out = run(&[]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("missing query"), "{}", stderr(&out));
}
