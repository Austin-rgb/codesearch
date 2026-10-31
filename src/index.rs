//! SQLite persistence and incremental indexing.
//!
//! The index lives in `<project>/.codesearch/index.db`. Two tables:
//!
//! * `files`: one row per `.rs` file with its size, mtime and BLAKE3 hash.
//! * `functions`: one row per extracted function. The parameter and return types
//!   are stored as JSON (the shared [`TypeExpr`](crate::types::TypeExpr) AST) plus
//!   two plain columns, `param_count` and `ret_head`, that SQLite can use to
//!   narrow candidates before any structural matching happens.
//!
//! Incremental update, per file:
//!
//! 1. size and mtime unchanged → skipped without reading the file;
//! 2. otherwise read + hash; hash unchanged → only the metadata is refreshed;
//! 3. otherwise re-parse and replace the file's rows.
//!
//! Files that disappeared are deleted (their functions go with them via `ON DELETE CASCADE`).

use crate::extractor;
use crate::types::{FnKind, FunctionSig};
use anyhow::{Context, Result};
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Directory (inside the project root) that holds the index.
pub const INDEX_DIR: &str = ".codesearch";
const DB_FILE: &str = "index.db";
/// Bump when the table layout or the stored JSON shape changes. A mismatch
/// rebuilds the index from scratch on the next run.
const SCHEMA_VERSION: &str = "1";

/// Directories that are never scanned.
const SKIP_DIRS: [&str; 4] = [".git", "target", "node_modules", INDEX_DIR];

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (
    id        INTEGER PRIMARY KEY,
    path      TEXT NOT NULL UNIQUE,
    hash      TEXT NOT NULL,
    size      INTEGER NOT NULL,
    mtime_ns  INTEGER NOT NULL,
    parse_ok  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS functions (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    module       TEXT NOT NULL,
    kind         TEXT NOT NULL,
    impl_target  TEXT,
    has_self     INTEGER NOT NULL,
    generics     TEXT NOT NULL,
    param_count  INTEGER NOT NULL,
    ret_head     TEXT,
    line         INTEGER NOT NULL,
    params_json  TEXT NOT NULL,
    ret_json     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_functions_sig  ON functions(param_count, ret_head);
CREATE INDEX IF NOT EXISTS idx_functions_file ON functions(file_id);
";

const INSERT_FUNCTION: &str = "
INSERT INTO functions
    (file_id, name, module, kind, impl_target, has_self, generics,
     param_count, ret_head, line, params_json, ret_json)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

/// What an [`Index::update`] run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Files (re)parsed in this run that `syn` could not parse. They are
    /// recorded with zero functions and retried when their content changes.
    pub parse_failures: usize,
}

impl UpdateReport {
    pub fn changed(&self) -> bool {
        self.added + self.modified + self.removed > 0
    }
}

/// Index-wide numbers for `codesearch stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    pub functions: usize,
    pub methods: usize,
    pub parse_failures: usize,
    pub db_path: PathBuf,
    pub db_size_bytes: u64,
}

/// Cheap, SQL-side narrowing of the candidates. The result is always a superset
/// of the true matches; the matcher makes the final decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFilter {
    pub param_count: usize,
    pub ret_head: Option<String>,
    /// Also accept methods with one more parameter (the `self` receiver).
    pub ignore_self: bool,
}

struct KnownFile {
    id: i64,
    hash: String,
    size: i64,
    mtime_ns: i64,
}

pub struct Index {
    conn: Connection,
    root: PathBuf,
    db_path: PathBuf,
}

impl Index {
    /// Opens (creating if necessary) the index for the project rooted at `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("project directory not found: {}", root.display()))?;
        if !root.is_dir() {
            anyhow::bail!("not a directory: {}", root.display());
        }

        let dir = root.join(INDEX_DIR);
        fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create index directory {}", dir.display()))?;
        // Keep the index out of version control without touching the project's own .gitignore.
        let gitignore = dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "*\n").ok();
        }

        let db_path = dir.join(DB_FILE);
        let conn = Connection::open(&db_path)
            .with_context(|| format!("cannot open index database {}", db_path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let current: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current.as_deref() != Some(SCHEMA_VERSION) {
            conn.execute_batch("DROP TABLE IF EXISTS functions; DROP TABLE IF EXISTS files;")?;
        }
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION],
        )?;

        Ok(Self { conn, root, db_path })
    }

    /// The canonical project root this index covers.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Brings the index in line with the files on disk.
    ///
    /// With `force`, every file is read and hashed even if its size and mtime look
    /// unchanged (content that is identical is still not re-parsed).
    pub fn update(&mut self, force: bool) -> Result<UpdateReport> {
        let mut report = UpdateReport::default();

        let mut known: HashMap<String, KnownFile> = HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT id, path, hash, size, mtime_ns FROM files")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    KnownFile {
                        id: row.get(0)?,
                        hash: row.get(2)?,
                        size: row.get(3)?,
                        mtime_ns: row.get(4)?,
                    },
                ))
            })?;
            for row in rows {
                let (path, file) = row?;
                known.insert(path, file);
            }
        }

        let discovered = discover_rust_files(&self.root);
        let mut seen: HashSet<String> = HashSet::with_capacity(discovered.len());

        let tx = self.conn.transaction()?;

        for (rel, abs) in discovered {
            let Ok(meta) = fs::metadata(&abs) else {
                continue; // vanished between the walk and now; treated as deleted below
            };
            let size = meta.len() as i64;
            let mtime_ns = mtime_nanos(&meta);
            seen.insert(rel.clone());

            let prior = known.get(&rel);
            if let Some(k) = prior {
                if !force && k.size == size && k.mtime_ns == mtime_ns {
                    report.unchanged += 1;
                    continue;
                }
            }

            let bytes = match fs::read(&abs) {
                Ok(bytes) => bytes,
                Err(_) => {
                    seen.remove(&rel);
                    continue;
                }
            };
            let hash = blake3::hash(&bytes).to_hex().to_string();

            if let Some(k) = prior {
                if k.hash == hash {
                    tx.execute(
                        "UPDATE files SET size = ?1, mtime_ns = ?2 WHERE id = ?3",
                        params![size, mtime_ns, k.id],
                    )?;
                    report.unchanged += 1;
                    continue;
                }
                // Changed content: drop the old rows (functions cascade) and re-insert.
                tx.execute("DELETE FROM files WHERE id = ?1", params![k.id])?;
                report.modified += 1;
            } else {
                report.added += 1;
            }

            let source = String::from_utf8_lossy(&bytes);
            let (functions, parse_ok) = match extractor::extract_functions(&source, &rel) {
                Ok(functions) => (functions, true),
                Err(_) => {
                    report.parse_failures += 1;
                    (Vec::new(), false)
                }
            };

            tx.execute(
                "INSERT INTO files (path, hash, size, mtime_ns, parse_ok)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![rel, hash, size, mtime_ns, i64::from(parse_ok)],
            )?;
            let file_id = tx.last_insert_rowid();

            let mut insert = tx.prepare_cached(INSERT_FUNCTION)?;
            for f in &functions {
                let has_self = i64::from(f.has_self);
                let generics = f.generics.join(",");
                let param_count = f.params.len() as i64;
                let ret_head = f.ret.head();
                let line = f.line as i64;
                let params_json = serde_json::to_string(&f.params)?;
                let ret_json = serde_json::to_string(&f.ret)?;
                insert.execute(params![
                    file_id,
                    f.name,
                    f.module,
                    f.kind.as_str(),
                    f.impl_target,
                    has_self,
                    generics,
                    param_count,
                    ret_head,
                    line,
                    params_json,
                    ret_json,
                ])?;
            }
        }

        for (path, file) in &known {
            if !seen.contains(path) {
                tx.execute("DELETE FROM files WHERE id = ?1", params![file.id])?;
                report.removed += 1;
            }
        }

        tx.commit()?;
        Ok(report)
    }

    /// Streams every function that passes the SQL pre-filter to `visit`, ordered by
    /// file and line. Memory use is independent of the number of candidates.
    pub fn for_each_candidate<F>(&self, filter: &CandidateFilter, mut visit: F) -> Result<()>
    where
        F: FnMut(FunctionSig) -> Result<()>,
    {
        let mut sql = String::from(
            "SELECT f.path, fu.name, fu.module, fu.kind, fu.impl_target, fu.has_self,
                    fu.generics, fu.line, fu.params_json, fu.ret_json
             FROM functions fu JOIN files f ON f.id = fu.file_id
             WHERE ",
        );
        let mut values = vec![Value::Integer(filter.param_count as i64)];

        if filter.ignore_self {
            sql.push_str("(fu.param_count = ?1 OR (fu.has_self = 1 AND fu.param_count = ?1 + 1))");
        } else {
            sql.push_str("fu.param_count = ?1");
        }
        if let Some(head) = &filter.ret_head {
            sql.push_str(" AND fu.ret_head = ?2");
            values.push(Value::Text(head.clone()));
        }
        sql.push_str(" ORDER BY f.path, fu.line");

        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values))?;
        while let Some(row) = rows.next()? {
            visit(row_to_signature(row)?)?;
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        Ok(Stats {
            files: self.count("SELECT COUNT(*) FROM files")?,
            functions: self.count("SELECT COUNT(*) FROM functions")?,
            methods: self.count("SELECT COUNT(*) FROM functions WHERE kind = 'method'")?,
            parse_failures: self.count("SELECT COUNT(*) FROM files WHERE parse_ok = 0")?,
            db_path: self.db_path.clone(),
            db_size_bytes: fs::metadata(&self.db_path).map(|m| m.len()).unwrap_or(0),
        })
    }

    fn count(&self, sql: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(sql, [], |row| row.get(0))?;
        Ok(n as usize)
    }
}

fn row_to_signature(row: &rusqlite::Row<'_>) -> Result<FunctionSig> {
    let generics: String = row.get(6)?;
    let params_json: String = row.get(8)?;
    let ret_json: String = row.get(9)?;
    let kind: String = row.get(3)?;
    let has_self: i64 = row.get(5)?;
    let line: i64 = row.get(7)?;

    Ok(FunctionSig {
        file: row.get(0)?,
        name: row.get(1)?,
        module: row.get(2)?,
        kind: FnKind::from_db(&kind),
        impl_target: row.get(4)?,
        has_self: has_self != 0,
        generics: if generics.is_empty() {
            Vec::new()
        } else {
            generics.split(',').map(str::to_string).collect()
        },
        line: line as usize,
        params: serde_json::from_str(&params_json)
            .context("corrupt index (parameters); delete .codesearch/ to rebuild")?,
        ret: serde_json::from_str(&ret_json)
            .context("corrupt index (return type); delete .codesearch/ to rebuild")?,
    })
}

fn mtime_nanos(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Returns `(relative path with '/' separators, absolute path)` for every `.rs`
/// file under `root`, skipping build output, VCS data and the index itself.
fn discover_rust_files(root: &Path) -> Vec<(String, PathBuf)> {
    let walker = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| *skip == &*name)
        });

    let mut files = Vec::new();
    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().map_or(true, |ext| ext != "rs") {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            files.push((rel, entry.path().to_path_buf()));
        }
    }
    files
}
