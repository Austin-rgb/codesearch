//! Discoverability linter (`codesearch lint`).
//!
//! Flags public functions whose signatures are hard to find through signature search or
//! easy to misuse: primitive-heavy parameter lists (R001) and missing or weak intent
//! lines in doc comments (R002).
//!
//! The linter reads function records from the SQLite index, refreshing it incrementally
//! first, exactly as `codesearch <query>` does; it never parses source files itself.
//!
//! # Adding a rule
//!
//! 1. Create `rules/rNNN.rs` with a type that implements [`Rule`]. `check` is a pure
//!    function of one [`FunctionSig`] and the [`LintConfig`]: no I/O.
//! 2. Add one line to `rules::all()` in `rules/mod.rs`.
//!
//! Opt-out comments, ignore globs, `--rule` selection, reporting and exit codes all work
//! on rule ids, so none of that code changes.

pub mod config;
pub mod report;
pub mod rules;

pub use config::LintConfig;
pub use report::{LintReport, Summary};

use crate::index::{Index, UpdateReport};
use crate::types::{FunctionSig, Visibility};
use anyhow::{bail, Result};
use serde::Serialize;
use std::path::Path;

/// How serious a finding is. Every rule currently reports [`Severity::Warn`];
/// `Info` is reserved for future advisory rules (extension point).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warn,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub file: String,
    pub line: usize,
    pub module: String,
    pub function: String,
    pub message: String,
    pub suggestion: String,
}

impl Finding {
    /// A warning located at `f`.
    pub fn warn(
        rule_id: &str,
        f: &FunctionSig,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Finding {
            rule_id: rule_id.to_string(),
            severity: Severity::Warn,
            file: f.file.clone(),
            line: f.line,
            module: f.module.clone(),
            function: f.name.clone(),
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }
}

/// A lint rule. Implementations must be pure: the same function and config always
/// produce the same findings, with no I/O.
pub trait Rule {
    /// Stable id such as `"R001"`, used in output, config, `--rule` and opt-out comments.
    fn id(&self) -> &'static str;
    fn check(&self, f: &FunctionSig, cfg: &LintConfig) -> Vec<Finding>;
}

/// Whether a function is nominally `pub` (not `pub(crate)`, `pub(super)`, `pub(in ..)`
/// or private). Rules that only apply to the public surface start with this check.
pub fn is_pub(f: &FunctionSig) -> bool {
    f.visibility == Visibility::Pub
}

/// Whether `// codesearch-lint: allow(..)` above the function names `rule_id`.
pub fn is_allowed(f: &FunctionSig, rule_id: &str) -> bool {
    f.lint_allow
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(rule_id))
}

/// Runs the enabled rules on one function and drops findings the function opted out of.
pub fn lint_function(rules: &[Box<dyn Rule>], f: &FunctionSig, cfg: &LintConfig) -> Vec<Finding> {
    let mut findings = Vec::new();
    for rule in rules {
        if cfg.is_enabled(rule.id()) && !is_allowed(f, rule.id()) {
            findings.extend(rule.check(f, cfg));
        }
    }
    findings
}

/// Checks the configured rule ids against the registry and returns the rules to run.
pub fn select_rules(available: Vec<Box<dyn Rule>>, cfg: &LintConfig) -> Result<Vec<Box<dyn Rule>>> {
    if let Some(wanted) = &cfg.enabled_rules {
        for id in wanted {
            if !available.iter().any(|r| r.id().eq_ignore_ascii_case(id)) {
                let known: Vec<&str> = available.iter().map(|r| r.id()).collect();
                bail!(
                    "unknown lint rule `{id}`; available rules: {}",
                    known.join(", ")
                );
            }
        }
    }
    Ok(available
        .into_iter()
        .filter(|rule| cfg.is_enabled(rule.id()))
        .collect())
}

/// Lints every function in `index` (minus files matching `cfg.ignore`).
///
/// `public_functions` counts nominally `pub` functions; `flagged_functions` counts those
/// with at least one finding, so a function with several findings counts once.
pub fn lint_index(index: &Index, rules: &[Box<dyn Rule>], cfg: &LintConfig) -> Result<LintReport> {
    let mut findings = Vec::new();
    let mut public_functions = 0;
    let mut flagged_functions = 0;

    index.for_each_function(|f| {
        if cfg.is_ignored(&f.file) {
            return Ok(());
        }
        let found = lint_function(rules, &f, cfg);
        if is_pub(&f) {
            public_functions += 1;
            if !found.is_empty() {
                flagged_functions += 1;
            }
        }
        findings.extend(found);
        Ok(())
    })?;

    let rule_ids: Vec<&str> = rules.iter().map(|r| r.id()).collect();
    Ok(LintReport::new(
        findings,
        public_functions,
        flagged_functions,
        &rule_ids,
    ))
}

/// Everything `codesearch lint` produces besides the exit code.
pub struct LintOutcome {
    pub report: LintReport,
    /// What the incremental index refresh did before linting.
    pub update: UpdateReport,
}

/// Opens the project's index, refreshes it, loads `.codesearch/lint.toml`, and lints.
/// A non-empty `rule_override` (from `--rule`) replaces `enabled_rules` from the config.
pub fn lint_project(project: &Path, rule_override: &[String]) -> Result<LintOutcome> {
    let mut index = Index::open(project)?;
    let update = index.update(false)?;

    let mut cfg = LintConfig::load(index.root())?;
    if !rule_override.is_empty() {
        cfg.enabled_rules = Some(
            rule_override
                .iter()
                .map(|id| id.trim().to_ascii_uppercase())
                .collect(),
        );
    }

    let rules = select_rules(rules::all(), &cfg)?;
    let report = lint_index(&index, &rules, &cfg)?;
    Ok(LintOutcome { report, update })
}
