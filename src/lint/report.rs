//! Lint results and their text / JSON renderings.
//!
//! Nothing here knows about individual rules: it works on [`Finding`]s and rule ids.

use super::Finding;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// Nominally `pub` functions that were linted (ignored files excluded).
    pub public_functions: usize,
    /// Public functions with at least one finding.
    pub flagged_functions: usize,
    pub total_findings: usize,
    /// Every rule that ran, including those with zero findings.
    pub findings_by_rule: BTreeMap<String, usize>,
    /// `1 - flagged_functions / public_functions`; `1.0` when there are no public functions.
    pub discoverability_score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LintReport {
    pub findings: Vec<Finding>,
    pub summary: Summary,
}

impl LintReport {
    /// Builds the report and its summary. `rule_ids` lists the rules that ran.
    pub fn new(
        findings: Vec<Finding>,
        public_functions: usize,
        flagged_functions: usize,
        rule_ids: &[&str],
    ) -> Self {
        let mut findings_by_rule: BTreeMap<String, usize> =
            rule_ids.iter().map(|id| (id.to_string(), 0)).collect();
        for finding in &findings {
            *findings_by_rule.entry(finding.rule_id.clone()).or_insert(0) += 1;
        }

        let discoverability_score = if public_functions == 0 {
            1.0
        } else {
            (public_functions - flagged_functions) as f64 / public_functions as f64
        };

        let summary = Summary {
            public_functions,
            flagged_functions,
            total_findings: findings.len(),
            findings_by_rule,
            discoverability_score,
        };
        LintReport { findings, summary }
    }

    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }
}

/// Findings grouped by file (blank line between files), then a summary.
pub fn render_text(report: &LintReport) -> String {
    let mut out = String::new();

    if report.findings.is_empty() {
        out.push_str("No discoverability findings.\n\n");
    }

    let mut current_file: Option<&str> = None;
    for finding in &report.findings {
        if current_file != Some(finding.file.as_str()) {
            if current_file.is_some() {
                out.push('\n');
            }
            current_file = Some(finding.file.as_str());
        }
        let _ = writeln!(
            out,
            "{}:{}  {}  {}",
            finding.file, finding.line, finding.rule_id, finding.message
        );
        let _ = writeln!(out, "    suggestion: {}", finding.suggestion);
    }
    if !report.findings.is_empty() {
        out.push('\n');
    }

    let s = &report.summary;
    let by_rule: Vec<String> = s
        .findings_by_rule
        .iter()
        .map(|(id, count)| format!("{id}: {count}"))
        .collect();
    let _ = writeln!(out, "Summary");
    let _ = writeln!(out, "  Public functions:   {}", s.public_functions);
    let _ = writeln!(out, "  Flagged functions:  {}", s.flagged_functions);
    let _ = writeln!(
        out,
        "  Findings:           {} ({})",
        s.total_findings,
        by_rule.join(", ")
    );
    let _ = writeln!(out, "  Discoverability:    {:.2}", s.discoverability_score);
    out
}

/// `{"findings": [...], "summary": {...}}`, pretty-printed.
pub fn render_json(report: &LintReport) -> serde_json::Result<String> {
    serde_json::to_string_pretty(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::Severity;

    fn finding(rule: &str, file: &str, line: usize) -> Finding {
        Finding {
            rule_id: rule.to_string(),
            severity: Severity::Warn,
            file: file.to_string(),
            line,
            module: "m".to_string(),
            function: "f".to_string(),
            message: "bad".to_string(),
            suggestion: "fix it".to_string(),
        }
    }

    #[test]
    fn score_is_one_without_public_functions() {
        let report = LintReport::new(Vec::new(), 0, 0, &["R001"]);
        assert_eq!(report.summary.discoverability_score, 1.0);
        assert!(!report.has_findings());
    }

    #[test]
    fn score_is_the_unflagged_fraction() {
        let report = LintReport::new(vec![finding("R001", "a.rs", 1)], 8, 3, &["R001"]);
        assert_eq!(report.summary.discoverability_score, 0.625);
        let all_bad = LintReport::new(Vec::new(), 4, 4, &[]);
        assert_eq!(all_bad.summary.discoverability_score, 0.0);
    }

    #[test]
    fn by_rule_counts_include_rules_with_no_findings() {
        let findings = vec![
            finding("R001", "a.rs", 1),
            finding("R001", "a.rs", 9),
            finding("R002", "b.rs", 3),
        ];
        let report = LintReport::new(findings, 5, 3, &["R001", "R002", "R003"]);
        let counts: Vec<_> = report
            .summary
            .findings_by_rule
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        assert_eq!(counts, vec![("R001", 2), ("R002", 1), ("R003", 0)]);
        assert_eq!(report.summary.total_findings, 3);
    }

    #[test]
    fn text_groups_findings_by_file() {
        let report = LintReport::new(
            vec![
                finding("R001", "a.rs", 4),
                finding("R002", "a.rs", 4),
                finding("R002", "b.rs", 7),
            ],
            6,
            2,
            &["R001", "R002"],
        );
        let text = render_text(&report);
        let expected_head = "\
a.rs:4  R001  bad
    suggestion: fix it
a.rs:4  R002  bad
    suggestion: fix it

b.rs:7  R002  bad
    suggestion: fix it

Summary
";
        assert!(text.starts_with(expected_head), "{text}");
        assert!(text.contains("  Public functions:   6\n"));
        assert!(text.contains("  Flagged functions:  2\n"));
        assert!(text.contains("  Findings:           3 (R001: 1, R002: 2)\n"));
        assert!(text.contains("  Discoverability:    0.67\n"));
    }

    #[test]
    fn text_without_findings_says_so() {
        let text = render_text(&LintReport::new(Vec::new(), 2, 0, &["R001"]));
        assert!(text.starts_with("No discoverability findings.\n\nSummary\n"));
        assert!(text.contains("Discoverability:    1.00"));
    }

    #[test]
    fn json_has_the_documented_shape() {
        let report = LintReport::new(vec![finding("R001", "a.rs", 4)], 2, 1, &["R001"]);
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&report).unwrap()).unwrap();
        assert_eq!(value["findings"][0]["rule_id"], "R001");
        assert_eq!(value["findings"][0]["severity"], "warn");
        assert_eq!(value["findings"][0]["line"], 4);
        assert_eq!(value["summary"]["public_functions"], 2);
        assert_eq!(value["summary"]["findings_by_rule"]["R001"], 1);
        assert_eq!(value["summary"]["discoverability_score"], 0.5);
    }
}
