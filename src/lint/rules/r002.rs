//! R002 `missing-intent-line`.
//!
//! A public function should open its doc comment with one short, complete sentence saying
//! what it does, followed by a blank doc line. Flagged when:
//!
//! * **(a)** there is no doc comment;
//! * **(b)** the first doc line is empty, shorter than `min_intent_len` characters
//!   (default 15), or only restates the function name (compared case- and
//!   underscore-insensitively);
//! * **(c)** the first doc line does not end with a period, or the first paragraph is longer
//!   than 200 characters.
//!
//! Decisions where the rule text leaves room:
//!
//! * Lengths count characters (not bytes) after trimming; exactly the minimum passes, and a
//!   paragraph of exactly 200 characters passes.
//! * The "first paragraph" is the doc lines up to the first blank doc line, joined with single
//!   spaces; leading blank doc lines are skipped.
//! * A sentence wrapped over two doc lines is judged on its first line, as the rule says.
//! * One finding per function; every violated clause is listed in its message.

use crate::lint::{is_pub, Finding, LintConfig, Rule};
use crate::types::FunctionSig;

const ID: &str = "R002";
const MAX_PARAGRAPH_CHARS: usize = 200;

pub struct MissingIntentLine;

impl Rule for MissingIntentLine {
    fn id(&self) -> &'static str {
        ID
    }

    fn check(&self, f: &FunctionSig, cfg: &LintConfig) -> Vec<Finding> {
        if !is_pub(f) {
            return Vec::new();
        }

        let suggestion = format!(
            "Add a first doc line such as: 'Does X given Y, returns Z.' \
             (one sentence of at least {} characters ending in a period, followed by a blank `///` line).",
            cfg.min_intent_len
        );

        let Some(doc) = &f.doc else {
            return vec![Finding::warn(
                ID,
                f,
                format!("public function `{}` has no doc comment", f.name),
                suggestion,
            )];
        };

        let first = match &f.doc_first_line {
            Some(line) => line.trim(),
            None => doc.split('\n').next().unwrap_or("").trim(),
        };

        let mut reasons: Vec<String> = Vec::new();
        if first.is_empty() {
            reasons.push("the first doc line is empty".to_string());
        } else {
            let len = first.chars().count();
            if len < cfg.min_intent_len {
                reasons.push(format!(
                    "the first doc line is too short ({len} characters, minimum {})",
                    cfg.min_intent_len
                ));
            }
            if normalize(first) == normalize(&f.name) {
                reasons.push("the first doc line only restates the function name".to_string());
            }
            if !first.ends_with('.') {
                reasons.push("the first doc line does not end with a period".to_string());
            }
        }

        let paragraph_len = first_paragraph(doc).chars().count();
        if paragraph_len > MAX_PARAGRAPH_CHARS {
            reasons.push(format!(
                "the first doc paragraph is {paragraph_len} characters (maximum {MAX_PARAGRAPH_CHARS}); \
                 keep the intent line to one short sentence followed by a blank doc line"
            ));
        }

        if reasons.is_empty() {
            return Vec::new();
        }
        vec![Finding::warn(
            ID,
            f,
            format!("weak intent line on `{}`: {}", f.name, reasons.join("; ")),
            suggestion,
        )]
    }
}

/// Lower-case alphanumerics only, so `Create session.` and `create_session` compare equal.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Doc lines up to the first blank one (ignoring leading blanks), joined with single spaces.
fn first_paragraph(doc: &str) -> String {
    doc.split('\n')
        .map(str::trim)
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::extract_functions;
    use crate::lint::lint_function;

    fn check_all(src: &str, cfg: &LintConfig) -> Vec<Finding> {
        extract_functions(src, "src/lib.rs")
            .unwrap()
            .iter()
            .flat_map(|f| MissingIntentLine.check(f, cfg))
            .collect()
    }

    fn flagged(src: &str) -> bool {
        !check_all(src, &LintConfig::default()).is_empty()
    }

    fn only(src: &str) -> Finding {
        let mut found = check_all(src, &LintConfig::default());
        assert_eq!(found.len(), 1, "expected exactly one finding: {found:?}");
        found.remove(0)
    }

    #[test]
    fn a_missing_doc_comment_is_flagged_with_the_template() {
        let finding = only("pub fn no_docs() {}");
        assert_eq!(finding.rule_id, "R002");
        assert!(finding.message.contains("`no_docs` has no doc comment"));
        assert!(finding
            .suggestion
            .starts_with("Add a first doc line such as: 'Does X given Y, returns Z.'"));
    }

    #[test]
    fn a_plain_comment_is_not_a_doc_comment() {
        assert!(flagged(
            "// Does something useful for callers.\npub fn f() {}"
        ));
        assert!(!flagged(
            "/// Does something useful for callers.\npub fn f() {}"
        ));
    }

    #[test]
    fn b_boundary_exactly_fifteen_characters_passes_and_fourteen_fails() {
        // "Parses a token." is 15 characters, "Reads a token." is 14.
        assert!(!flagged("/// Parses a token.\npub fn parse_input() {}"));
        let finding = only("/// Reads a token.\npub fn read_input() {}");
        assert!(finding
            .message
            .contains("too short (14 characters, minimum 15)"));
    }

    #[test]
    fn b_an_empty_first_line_is_flagged() {
        let finding = only("///\n/// Starts with a blank line.\npub fn blank_first() {}");
        assert!(finding.message.contains("the first doc line is empty"));
        assert!(flagged("/// \npub fn only_whitespace() {}"));
    }

    #[test]
    fn b_restating_the_function_name_is_flagged() {
        let finding = only("/// Create session.\npub fn create_session() {}");
        assert!(finding.message.contains("only restates the function name"));
        assert!(!finding.message.contains("too short"));
        assert!(flagged("/// CREATE_SESSION.\npub fn create_session() {}"));
        // A longer sentence that merely contains the name is fine.
        assert!(!flagged(
            "/// Create session tokens for a user.\npub fn create_session() {}"
        ));
    }

    #[test]
    fn c_a_missing_final_period_is_flagged() {
        let finding =
            only("/// Closes the connection and flushes pending writes\npub fn close() {}");
        assert!(finding.message.contains("does not end with a period"));
        assert!(flagged(
            "/// Closes the connection and flushes!\npub fn close() {}"
        ));
    }

    #[test]
    fn c_boundary_paragraph_of_exactly_200_characters_passes() {
        let ok = format!("{}.", "x".repeat(199));
        assert_eq!(ok.chars().count(), 200);
        assert!(!flagged(&format!("/// {ok}\npub fn f() {{}}")));

        let too_long = format!("{}.", "x".repeat(200));
        let finding = only(&format!("/// {too_long}\npub fn f() {{}}"));
        assert!(finding
            .message
            .contains("first doc paragraph is 201 characters"));
    }

    #[test]
    fn c_the_paragraph_ends_at_the_first_blank_doc_line() {
        let long = "y".repeat(170);
        // Wrapped onto a second line: one paragraph, too long.
        let wrapped =
            format!("/// Opens the store and returns a handle.\n/// {long}\npub fn f() {{}}");
        assert!(only(&wrapped)
            .message
            .contains("first doc paragraph is 208 characters"));
        // The same text after a blank doc line is a separate paragraph: fine.
        let split =
            format!("/// Opens the store and returns a handle.\n///\n/// {long}\npub fn f() {{}}");
        assert!(!flagged(&split));
    }

    #[test]
    fn well_formed_docs_pass() {
        assert!(!flagged(
            "/// Opens the store at the given location and returns a handle.\n\
             ///\n\
             /// Fails when the location is not writable.\n\
             pub fn open_store() {}"
        ));
        assert!(!flagged(
            "#[doc = \"Hello there, this is fine.\"]\npub fn f() {}"
        ));
        assert!(!flagged("/** Hello there, this is fine. */\npub fn f() {}"));
    }

    #[test]
    fn several_violations_make_one_finding() {
        let finding = only("/// Hi\npub fn f() {}");
        assert!(finding.message.contains("too short (2 characters"));
        assert!(finding.message.contains("does not end with a period"));
    }

    #[test]
    fn only_plain_pub_functions_are_checked() {
        for vis in ["pub(crate)", "pub(super)", "pub(in crate::a)", ""] {
            assert!(
                !flagged(&format!("{vis} fn undocumented() {{}}")),
                "`{vis}`"
            );
        }
        assert!(flagged("pub fn undocumented() {}"));
    }

    #[test]
    fn methods_are_checked_too() {
        assert!(flagged(
            "struct S; impl S { pub fn undocumented(&self) {} }"
        ));
        assert!(!flagged(
            "struct S; impl S {\n/// Returns the number of stored items.\npub fn len(&self) -> usize { 0 }\n}"
        ));
    }

    #[test]
    fn the_minimum_length_is_configurable() {
        let cfg = LintConfig {
            min_intent_len: 5,
            ..LintConfig::default()
        };
        assert!(check_all("/// Hi all.\npub fn f() {}", &cfg).is_empty());
        assert_eq!(
            check_all("/// Hi all.\npub fn f() {}", &LintConfig::default()).len(),
            1
        );
        let strict = LintConfig {
            min_intent_len: 40,
            ..LintConfig::default()
        };
        assert_eq!(
            check_all("/// Parses a token.\npub fn parse_input() {}", &strict).len(),
            1
        );
    }

    #[test]
    fn the_opt_out_comment_is_honoured() {
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(MissingIntentLine)];
        let cfg = LintConfig::default();

        let plain = extract_functions("pub fn f() {}", "src/lib.rs").unwrap();
        assert_eq!(lint_function(&rules, &plain[0], &cfg).len(), 1);

        for src in [
            "// codesearch-lint: allow(R002)\npub fn f() {}",
            "// codesearch-lint: allow(R001, R002)\npub fn f() {}",
            "// unrelated note\n// codesearch-lint: allow(R002)\n#[inline]\npub fn f() {}",
            "#[inline]\n// codesearch-lint: allow(R002)\npub fn f() {}",
        ] {
            let sigs = extract_functions(src, "src/lib.rs").unwrap();
            assert!(lint_function(&rules, &sigs[0], &cfg).is_empty(), "{src}");
        }

        // A blank line breaks "directly above"; so does naming another rule.
        for src in [
            "// codesearch-lint: allow(R002)\n\npub fn f() {}",
            "// codesearch-lint: allow(R001)\npub fn f() {}",
        ] {
            let sigs = extract_functions(src, "src/lib.rs").unwrap();
            assert_eq!(lint_function(&rules, &sigs[0], &cfg).len(), 1, "{src}");
        }
    }

    #[test]
    fn the_opt_out_does_not_leak_to_neighbouring_functions() {
        let src = "// codesearch-lint: allow(R002)\npub fn quiet() {}\npub fn loud() {}";
        let findings = check_all(src, &LintConfig::default());
        // `check` itself ignores opt-outs; `lint_function` applies them.
        assert_eq!(findings.len(), 2);

        let rules: Vec<Box<dyn Rule>> = vec![Box::new(MissingIntentLine)];
        let sigs = extract_functions(src, "src/lib.rs").unwrap();
        let cfg = LintConfig::default();
        assert!(lint_function(&rules, &sigs[0], &cfg).is_empty());
        assert_eq!(lint_function(&rules, &sigs[1], &cfg).len(), 1);
    }
}
