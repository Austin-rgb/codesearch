//! Lint configuration: defaults, the optional `.codesearch/lint.toml`, and ignore globs.
//!
//! ```toml
//! # .codesearch/lint.toml: every key is optional
//! enabled_rules  = ["R001", "R002"]          # default: every registered rule
//! domain_words   = ["id", "email", "sku"]    # replaces the default list
//! min_intent_len = 15                        # R002: minimum first-doc-line length
//! ignore         = ["tests/**", "**/generated/**", "*_test.rs"]
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

/// Config file location, relative to the project root.
pub const CONFIG_PATH: &str = ".codesearch/lint.toml";

pub const DEFAULT_DOMAIN_WORDS: &[&str] = &[
    "id",
    "uid",
    "email",
    "name",
    "username",
    "password",
    "token",
    "key",
    "url",
    "uri",
    "path",
    "user",
    "tenant",
    "session",
    "amount",
    "currency",
    "timestamp",
];

pub const DEFAULT_MIN_INTENT_LEN: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintConfig {
    /// Rule ids to run (upper-case). `None` means every registered rule.
    pub enabled_rules: Option<Vec<String>>,
    /// Lower-case words that mark a primitive as a domain value (R001b).
    pub domain_words: Vec<String>,
    /// R002: the first doc line must have at least this many characters.
    pub min_intent_len: usize,
    /// Glob patterns for files to leave out of linting entirely.
    pub ignore: Vec<String>,
}

impl Default for LintConfig {
    fn default() -> Self {
        LintConfig {
            enabled_rules: None,
            domain_words: DEFAULT_DOMAIN_WORDS.iter().map(|w| w.to_string()).collect(),
            min_intent_len: DEFAULT_MIN_INTENT_LEN,
            ignore: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    enabled_rules: Option<Vec<String>>,
    domain_words: Option<Vec<String>>,
    min_intent_len: Option<usize>,
    ignore: Option<Vec<String>>,
}

impl LintConfig {
    /// Parses the contents of a `lint.toml`. Unknown keys are an error so typos are caught.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let raw: RawConfig = toml::from_str(text)?;
        let defaults = LintConfig::default();
        Ok(LintConfig {
            enabled_rules: raw.enabled_rules.map(|rules| {
                rules
                    .iter()
                    .map(|r| r.trim().to_ascii_uppercase())
                    .collect()
            }),
            domain_words: raw.domain_words.map_or(defaults.domain_words, |words| {
                words
                    .iter()
                    .map(|w| w.trim().to_ascii_lowercase())
                    .collect()
            }),
            min_intent_len: raw.min_intent_len.unwrap_or(defaults.min_intent_len),
            ignore: raw.ignore.unwrap_or(defaults.ignore),
        })
    }

    /// Loads `<root>/.codesearch/lint.toml`, or the defaults when the file does not exist.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_PATH);
        match fs::read_to_string(&path) {
            Ok(text) => Self::from_toml(&text)
                .with_context(|| format!("invalid lint config {}", path.display())),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => {
                Err(err).with_context(|| format!("cannot read lint config {}", path.display()))
            }
        }
    }

    pub fn is_enabled(&self, rule_id: &str) -> bool {
        match &self.enabled_rules {
            None => true,
            Some(rules) => rules.iter().any(|r| r.eq_ignore_ascii_case(rule_id)),
        }
    }

    /// True when `rel_path` (relative to the project root, `/`-separated) matches an ignore glob.
    pub fn is_ignored(&self, rel_path: &str) -> bool {
        self.ignore
            .iter()
            .any(|pattern| glob_match(pattern, rel_path))
    }

    /// True when any `_`-separated token of `name` is a domain word (or its plural).
    /// `user_id` has tokens `user` and `id`; `paid` and `monkey` match nothing.
    pub fn has_domain_word(&self, name: &str) -> bool {
        name_tokens(name).iter().any(|token| {
            self.domain_words.iter().any(|word| {
                token == word || (token.len() == word.len() + 1 && token == &format!("{word}s"))
            })
        })
    }
}

/// Lower-case alphanumeric tokens of a snake_case name: `get_user_ID` → `get`, `user`, `id`.
pub fn name_tokens(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Minimal glob matcher over `/`-separated relative paths.
///
/// * `*` matches any run of characters except `/`; `?` matches one such character.
/// * `**` matches across directories; `**/` also matches zero directories.
/// * A pattern without `/` is matched against the file name only (`*_test.rs`).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let target = if pattern.contains('/') {
        path
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };
    glob_bytes(pattern.as_bytes(), target.as_bytes())
}

fn glob_bytes(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) if rest.first() == Some(&b'*') => {
            let mut rest = &rest[1..];
            let slash = rest.first() == Some(&b'/');
            if slash {
                rest = &rest[1..];
            }
            (0..=text.len()).any(|i| {
                // `**/` may only start matching at the beginning or right after a `/`.
                (!slash || i == 0 || text[i - 1] == b'/') && glob_bytes(rest, &text[i..])
            })
        }
        Some((b'*', rest)) => {
            for i in 0..=text.len() {
                if glob_bytes(rest, &text[i..]) {
                    return true;
                }
                if text.get(i) == Some(&b'/') || i == text.len() {
                    return false;
                }
            }
            false
        }
        Some((b'?', rest)) => match text.split_first() {
            Some((c, tail)) if *c != b'/' => glob_bytes(rest, tail),
            _ => false,
        },
        Some((p, rest)) => match text.split_first() {
            Some((c, tail)) if c == p => glob_bytes(rest, tail),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_work_without_a_file() {
        let cfg = LintConfig::default();
        assert!(cfg.is_enabled("R001") && cfg.is_enabled("r002"));
        assert_eq!(cfg.min_intent_len, 15);
        assert_eq!(cfg.domain_words.len(), 17);
        assert!(cfg.ignore.is_empty());
    }

    #[test]
    fn parses_every_key() {
        let cfg = LintConfig::from_toml(
            r#"
            enabled_rules = ["r001"]
            domain_words = ["SKU", "id"]
            min_intent_len = 20
            ignore = ["tests/**"]
            "#,
        )
        .unwrap();
        assert!(cfg.is_enabled("R001") && !cfg.is_enabled("R002"));
        assert_eq!(cfg.domain_words, vec!["sku", "id"]);
        assert_eq!(cfg.min_intent_len, 20);
        assert_eq!(cfg.ignore, vec!["tests/**"]);
    }

    #[test]
    fn partial_files_keep_the_other_defaults() {
        let cfg = LintConfig::from_toml("min_intent_len = 5").unwrap();
        assert_eq!(cfg.min_intent_len, 5);
        assert_eq!(cfg.domain_words.len(), 17);
        assert!(cfg.enabled_rules.is_none());
    }

    #[test]
    fn unknown_keys_and_bad_types_are_errors() {
        assert!(LintConfig::from_toml("min_intent = 5").is_err());
        assert!(LintConfig::from_toml("min_intent_len = \"long\"").is_err());
    }

    #[test]
    fn missing_file_gives_defaults_and_a_present_file_is_read() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(LintConfig::load(dir.path()).unwrap(), LintConfig::default());

        fs::create_dir_all(dir.path().join(".codesearch")).unwrap();
        fs::write(dir.path().join(CONFIG_PATH), "min_intent_len = 9").unwrap();
        assert_eq!(LintConfig::load(dir.path()).unwrap().min_intent_len, 9);

        fs::write(dir.path().join(CONFIG_PATH), "bogus = 1").unwrap();
        let err = LintConfig::load(dir.path()).unwrap_err();
        assert!(format!("{err:#}").contains("invalid lint config"));
    }

    #[test]
    fn domain_words_match_whole_tokens_and_plurals() {
        let cfg = LintConfig::default();
        assert!(cfg.has_domain_word("user_id"));
        assert!(cfg.has_domain_word("_email"));
        assert!(cfg.has_domain_word("get_Token"));
        assert!(cfg.has_domain_word("emails"));
        assert!(!cfg.has_domain_word("paid"));
        assert!(!cfg.has_domain_word("monkey"));
        assert!(!cfg.has_domain_word("identity"));
        assert!(!cfg.has_domain_word("count"));
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("tests/**", "tests/a/b.rs"));
        assert!(glob_match("tests/**", "tests/a.rs"));
        assert!(!glob_match("tests/**", "src/tests/a.rs"));
        assert!(glob_match("**/generated/**", "generated/x.rs"));
        assert!(glob_match("**/generated/**", "src/deep/generated/x.rs"));
        assert!(!glob_match("**/generated/**", "src/not_generated/x.rs"));
        assert!(glob_match("src/*.rs", "src/lib.rs"));
        assert!(!glob_match("src/*.rs", "src/a/lib.rs"));
        assert!(glob_match("src/?.rs", "src/a.rs"));
        assert!(!glob_match("src/?.rs", "src/ab.rs"));
    }

    #[test]
    fn patterns_without_a_slash_match_the_file_name() {
        assert!(glob_match("*_test.rs", "src/deep/user_test.rs"));
        assert!(glob_match("build.rs", "build.rs"));
        assert!(!glob_match("build.rs", "src/rebuild.rs"));
    }

    #[test]
    fn ignore_uses_the_globs() {
        let cfg = LintConfig {
            ignore: vec!["examples/**".into(), "*_gen.rs".into()],
            ..LintConfig::default()
        };
        assert!(cfg.is_ignored("examples/a.rs"));
        assert!(cfg.is_ignored("src/x_gen.rs"));
        assert!(!cfg.is_ignored("src/x.rs"));
    }
}
