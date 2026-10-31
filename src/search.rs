//! Coordinates a search (open index → refresh → pre-filter → match) and formats output.

use crate::index::{CandidateFilter, Index, Stats, UpdateReport};
use crate::matcher::{MatchOptions, Matcher};
use crate::types::{FunctionSig, Query};
use anyhow::Result;
use std::fmt::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// See [`MatchOptions::ignore_self`].
    pub ignore_self: bool,
    /// Refresh the index (cheap, incremental) before searching.
    pub update_index: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            ignore_self: false,
            update_index: true,
        }
    }
}

pub struct SearchOutcome {
    pub results: Vec<FunctionSig>,
    /// `Some` when the index was refreshed before searching.
    pub update: Option<UpdateReport>,
}

/// Opens (initialising if needed) the project's index, refreshes it, and searches it.
pub fn search_project(
    project: &Path,
    query: &Query,
    opts: &SearchOptions,
) -> Result<SearchOutcome> {
    let mut index = Index::open(project)?;
    let update = if opts.update_index {
        Some(index.update(false)?)
    } else {
        None
    };
    let results = search(
        &index,
        query,
        &Matcher::default(),
        MatchOptions {
            ignore_self: opts.ignore_self,
        },
    )?;
    Ok(SearchOutcome { results, update })
}

/// Searches an already-open index. Candidates are narrowed in SQL by parameter
/// count and return-type head, then confirmed by the structural matcher.
pub fn search(
    index: &Index,
    query: &Query,
    matcher: &Matcher,
    opts: MatchOptions,
) -> Result<Vec<FunctionSig>> {
    let filter = CandidateFilter {
        param_count: query.params.len(),
        ret_head: query.ret.head(),
        ignore_self: opts.ignore_self,
    };

    let mut results = Vec::new();
    index.for_each_candidate(&filter, |candidate| {
        if matcher.matches(query, &candidate, opts) {
            results.push(candidate);
        }
        Ok(())
    })?;
    Ok(results)
}

pub fn format_results(results: &[FunctionSig]) -> String {
    if results.is_empty() {
        return "No matching functions found.\n".to_string();
    }

    let count = results.len();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Found {count} matching function{}:\n",
        if count == 1 { "" } else { "s" }
    );

    for (i, f) in results.iter().enumerate() {
        let _ = writeln!(out, "{}. {}", i + 1, f.name);
        let _ = writeln!(out, "   Signature: {}", f.signature_string());
        let _ = writeln!(out, "   File: {}", f.file);
        let _ = writeln!(out, "   Line: {}", f.line);
        if !f.module.is_empty() {
            let _ = writeln!(out, "   Module: {}", f.module);
        }
        if f.impl_target.is_some() {
            let _ = writeln!(out, "   Kind: {}", f.kind_label());
        }
        out.push('\n');
    }

    let _ = writeln!(
        out,
        "{count} match{} found.",
        if count == 1 { "" } else { "es" }
    );
    out
}

pub fn format_index_summary(root: &Path, report: &UpdateReport, stats: &Stats) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Indexed {}", root.display());
    let _ = writeln!(
        out,
        "  Files:     {} added, {} modified, {} removed, {} unchanged",
        report.added, report.modified, report.removed, report.unchanged
    );
    let _ = writeln!(
        out,
        "  Functions: {} in {} files",
        stats.functions, stats.files
    );
    if report.parse_failures > 0 {
        let _ = writeln!(
            out,
            "  Skipped:   {} file(s) could not be parsed",
            report.parse_failures
        );
    }
    out
}

pub fn format_stats(stats: &Stats) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Index:         {}", stats.db_path.display());
    let _ = writeln!(out, "Size:          {}", format_bytes(stats.db_size_bytes));
    let _ = writeln!(out, "Files:         {}", stats.files);
    let _ = writeln!(
        out,
        "Functions:     {} ({} methods)",
        stats.functions, stats.methods
    );
    let _ = writeln!(out, "Unparsable:    {} file(s)", stats.parse_failures);
    out
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{:.1} MiB", b / (KIB * KIB))
    }
}
