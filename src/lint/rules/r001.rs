//! R001 `primitive-heavy-signature`.
//!
//! A public function is flagged when its signature leans on bare primitives in a way that
//! makes it easy to misuse and hard to find with a signature search:
//!
//! * **(a)** two or more parameters share the same bare primitive type;
//! * **(b)** a parameter (or the return type) is a bare primitive and the parameter name
//!   (or function name, for the return type) contains a domain word;
//! * **(c)** three or more parameters, all bare primitives.
//!
//! Decisions where the rule text leaves room:
//!
//! * "Bare primitive" means `String`, `str`, `bool`, `char`, the integer and float types,
//!   `usize` and `isize`, optionally behind references (`&str`, `&mut u8`). `std::string::String`
//!   written with a path is not recognised.
//! * "Same type" in (a) is the written type: `String` and `&str` differ, `&str` and `&str` do not.
//! * A `self` receiver is not a parameter for (a) and (c).
//! * Primitives inside generics (`Vec<u8>`, `Option<String>`) only count for (b).
//! * Domain words match whole `_`-separated tokens (plus a plural `s`), not substrings.
//! * Methods in trait impls are skipped: their signature is dictated by the trait.
//!   (The extractor cannot tell local traits from foreign ones, and trait-impl methods cannot be
//!   written `pub` anyway.)
//! * One finding per function; all triggered clauses are listed in its message.

use crate::lint::config::name_tokens;
use crate::lint::{is_pub, Finding, LintConfig, Rule};
use crate::types::{FunctionSig, Param, TypeExpr};
use std::collections::BTreeMap;

const ID: &str = "R001";

const PRIMITIVES: &[&str] = &[
    "String", "str", "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
    "u32", "u64", "u128", "usize", "f32", "f64",
];

/// Leading verbs dropped when deriving a newtype name from a function name.
const VERBS: &[&str] = &[
    "get",
    "set",
    "find",
    "fetch",
    "load",
    "read",
    "parse",
    "create",
    "new",
    "make",
    "build",
    "generate",
    "lookup",
    "resolve",
    "compute",
    "calculate",
    "is",
    "has",
    "to",
    "into",
    "from",
    "as",
    "with",
    "list",
];

/// Parameter names too generic to name a newtype on their own (`id` → `UserId`).
const GENERIC_NAMES: &[&str] = &["id", "uid", "key", "name", "token"];

pub struct PrimitiveHeavySignature;

impl Rule for PrimitiveHeavySignature {
    fn id(&self) -> &'static str {
        ID
    }

    fn check(&self, f: &FunctionSig, cfg: &LintConfig) -> Vec<Finding> {
        if !is_pub(f) || f.trait_impl.is_some() {
            return Vec::new();
        }

        let params: Vec<&Param> = f.params.iter().skip(usize::from(f.has_self)).collect();
        let mut reasons: Vec<String> = Vec::new();
        let mut suggestions: Vec<String> = Vec::new();

        // (b) is computed first so (a)'s suggestion can skip parameters (b) already covers.
        let domain_params: Vec<&Param> = params
            .iter()
            .copied()
            .filter(|p| mentions_primitive(&p.ty) && cfg.has_domain_word(&p.name))
            .collect();
        let domain_return = mentions_primitive(&f.ret) && cfg.has_domain_word(&f.name);

        // (a) duplicated bare primitive types.
        let mut by_type: BTreeMap<String, Vec<&Param>> = BTreeMap::new();
        for p in params.iter().copied().filter(|p| is_bare_primitive(&p.ty)) {
            by_type.entry(p.ty.to_string()).or_default().push(p);
        }
        for (ty, group) in by_type.iter().filter(|(_, group)| group.len() >= 2) {
            let names: Vec<&str> = group.iter().map(|p| p.name.as_str()).collect();
            reasons.push(format!(
                "parameters {} share the primitive type `{ty}`",
                join_names(&names)
            ));
            let uncovered: Vec<&str> = group
                .iter()
                .filter(|p| !domain_params.iter().any(|d| d.name == p.name))
                .map(|p| p.name.as_str())
                .collect();
            if !uncovered.is_empty() {
                suggestions.push(format!(
                    "Consider distinct newtypes (or a parameter struct) for {}, which share the type `{ty}`.",
                    join_names(&uncovered)
                ));
            }
        }

        // (b) domain-named primitives.
        if !domain_params.is_empty() {
            let shown: Vec<String> = domain_params
                .iter()
                .map(|p| format!("`{}: {}`", p.name, p.ty))
                .collect();
            reasons.push(if shown.len() == 1 {
                format!(
                    "{} looks like a domain value passed as a bare primitive",
                    shown[0]
                )
            } else {
                format!(
                    "{} look like domain values passed as bare primitives",
                    join_list(&shown)
                )
            });
            for p in &domain_params {
                suggestions.push(format!(
                    "Consider a newtype such as `{}` for parameter `{}: {}`.",
                    newtype_for_param(&p.name, &f.name),
                    p.name,
                    p.ty
                ));
            }
        }
        if domain_return {
            reasons.push(format!(
                "the return type `{}` of `{}` looks like a domain value",
                f.ret, f.name
            ));
            suggestions.push(format!(
                "Consider a newtype such as `{}` for the return value of `{}`.",
                newtype_for_function(&f.name),
                f.name
            ));
        }

        // (c) three or more parameters, all bare primitives.
        if params.len() >= 3 && params.iter().all(|p| is_bare_primitive(&p.ty)) {
            reasons.push(format!(
                "all {} parameters are bare primitives",
                params.len()
            ));
            if suggestions.is_empty() {
                suggestions.push(
                    "Consider a parameter struct or newtypes so each argument is labelled by its type."
                        .to_string(),
                );
            }
        }

        if reasons.is_empty() {
            return Vec::new();
        }
        vec![Finding::warn(
            ID,
            f,
            format!(
                "primitive-heavy signature in `{}`: {}",
                f.name,
                reasons.join("; ")
            ),
            suggestions.join(" "),
        )]
    }
}

/// A bare primitive, looking through references: `u64`, `&str`, `&mut String`.
fn is_bare_primitive(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Ref { inner, .. } => is_bare_primitive(inner),
        TypeExpr::Path { segments, args } => {
            args.is_empty() && segments.len() == 1 && PRIMITIVES.contains(&segments[0].as_str())
        }
        _ => false,
    }
}

/// A bare primitive, or one nested anywhere in generic arguments (`Option<String>`).
fn mentions_primitive(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Ref { inner, .. } => mentions_primitive(inner),
        TypeExpr::Path { args, .. } => is_bare_primitive(ty) || args.iter().any(mentions_primitive),
        _ => false,
    }
}

/// `"a"`, `"a and b"`, `"a, b and c"`, each name backticked.
fn join_names(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    join_list(&quoted)
}

fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn camel_case(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| {
            let mut chars = t.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// `user_id` → `UserId`. A bare `id` / `key` / `name` borrows a noun from the function name:
/// `get_user(id)` → `UserId`.
fn newtype_for_param(param: &str, function: &str) -> String {
    let mut tokens = name_tokens(param);
    if tokens.len() == 1 && GENERIC_NAMES.contains(&tokens[0].as_str()) {
        let own = tokens[0].clone();
        let context = name_tokens(function)
            .into_iter()
            .find(|t| !VERBS.contains(&t.as_str()) && *t != own);
        if let Some(context) = context {
            tokens.insert(0, context);
        }
    }
    camel_case(&tokens)
}

/// `get_user_id` → `UserId`.
fn newtype_for_function(function: &str) -> String {
    let tokens: Vec<String> = name_tokens(function)
        .into_iter()
        .filter(|t| !VERBS.contains(&t.as_str()))
        .collect();
    if tokens.is_empty() {
        "Value".to_string()
    } else {
        camel_case(&tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::extract_functions;
    use crate::lint::{lint_function, Severity};
    use crate::types::Visibility;

    fn check_all(src: &str, cfg: &LintConfig) -> Vec<Finding> {
        extract_functions(src, "src/lib.rs")
            .unwrap()
            .iter()
            .flat_map(|f| PrimitiveHeavySignature.check(f, cfg))
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
    fn a_two_params_of_the_same_primitive_are_flagged() {
        let finding = only("pub fn f(left: u64, right: u64) {}");
        assert_eq!(finding.rule_id, "R001");
        assert_eq!(finding.severity, Severity::Warn);
        assert!(finding
            .message
            .contains("parameters `left` and `right` share the primitive type `u64`"));
        assert!(finding.suggestion.contains("`left` and `right`"));
        assert_eq!(finding.line, 1);
    }

    #[test]
    fn a_three_way_duplicate_lists_every_name() {
        let finding = only("pub fn f(a: String, b: String, c: String) {}");
        assert!(finding.message.contains("`a`, `b` and `c` share"));
    }

    #[test]
    fn a_boundary_different_primitives_are_not_flagged() {
        assert!(!flagged("pub fn f(a: u64, b: String) -> bool { true }"));
        assert!(!flagged("pub fn f(a: u32, b: u64) {}"));
        assert!(!flagged("pub fn f(a: f32, b: f64) {}"));
    }

    #[test]
    fn a_owned_and_borrowed_strings_are_different_types() {
        assert!(!flagged("pub fn f(a: String, b: &str) {}"));
        assert!(!flagged("pub fn f(a: &str, b: &mut str) {}"));
        assert!(flagged("pub fn f(a: &str, b: &str) {}"));
    }

    #[test]
    fn a_non_primitive_duplicates_are_fine() {
        assert!(!flagged("pub fn f(a: Foo, b: Foo) {}"));
        assert!(!flagged("pub fn f(a: Vec<u8>, b: Vec<u8>) {}"));
    }

    #[test]
    fn b_domain_named_primitive_parameter_is_flagged_with_the_spec_suggestion() {
        let finding = only("pub fn lookup(user_id: u64) -> Session { todo!() }");
        assert!(finding
            .message
            .contains("`user_id: u64` looks like a domain value"));
        assert_eq!(
            finding.suggestion,
            "Consider a newtype such as `UserId` for parameter `user_id: u64`."
        );
    }

    #[test]
    fn b_generic_names_borrow_context_from_the_function_name() {
        let finding = only("pub fn get_user(id: u64) -> User { todo!() }");
        assert!(finding
            .suggestion
            .contains("`UserId` for parameter `id: u64`"));
        let finding = only("pub fn load(id: u64) -> User { todo!() }");
        assert!(finding.suggestion.contains("`Id` for parameter `id: u64`"));
    }

    #[test]
    fn b_domain_returns_are_judged_by_the_function_name() {
        let finding = only("pub fn get_user_id(name: UserName) -> u64 { 0 }");
        assert!(finding
            .message
            .contains("the return type `u64` of `get_user_id` looks like a domain value"));
        assert!(finding
            .suggestion
            .contains("`UserId` for the return value of `get_user_id`"));
        assert!(!flagged("pub fn checksum(data: Data) -> u32 { 0 }"));
    }

    #[test]
    fn b_matches_whole_name_tokens_only() {
        assert!(!flagged("pub fn f(paid: bool) {}"));
        assert!(!flagged("pub fn f(monkey: String) {}"));
        assert!(!flagged("pub fn f(identity: String) {}"));
        assert!(flagged("pub fn f(emails: String) {}"));
        assert!(flagged("pub fn f(_token: String) {}"));
    }

    #[test]
    fn b_bare_domain_name_with_a_non_primitive_type_is_fine() {
        assert!(!flagged("pub fn f(user_id: UserId) {}"));
        assert!(!flagged("pub fn f(user: &User) -> Session { todo!() }"));
    }

    #[test]
    fn single_parameter_with_a_non_domain_name_is_not_flagged() {
        assert!(!flagged("pub fn f(count: u32) -> u32 { count }"));
        assert!(!flagged("pub fn f(flag: bool) {}"));
        assert!(!flagged("pub fn f() -> u32 { 0 }"));
    }

    #[test]
    fn primitives_inside_generics_are_only_flagged_through_b() {
        assert!(!flagged(
            "pub fn f(data: Vec<u8>, extra: Option<String>) {}"
        ));
        assert!(!flagged("pub fn f(a: Vec<u8>, b: Vec<u8>, c: Vec<u8>) {}"));
        assert!(flagged("pub fn f(email: Option<String>) {}"));
        assert!(flagged(
            "pub fn find_user_email() -> Result<String, Error> { todo!() }"
        ));
        assert!(!flagged("pub fn f(token: Vec<Foo>) {}"));
    }

    #[test]
    fn c_three_distinct_primitives_are_flagged_only_by_the_all_primitive_clause() {
        let finding = only("pub fn f(a: u8, b: u16, c: u32) {}");
        assert!(finding
            .message
            .contains("all 3 parameters are bare primitives"));
        assert!(!finding.message.contains("share"));
        assert!(finding.suggestion.contains("parameter struct"));
    }

    #[test]
    fn c_boundary_two_params_or_one_non_primitive_are_not_flagged() {
        assert!(!flagged("pub fn f(a: u8, b: u16) {}"));
        assert!(!flagged("pub fn f(a: u8, b: u16, c: Foo) {}"));
        assert!(!flagged("pub fn f(a: u8, b: u16, c: Vec<u8>) {}"));
    }

    #[test]
    fn several_clauses_produce_one_finding() {
        let finding = only("pub fn transfer(from: u64, to: u64, amount: u64) -> bool { true }");
        assert!(finding.message.contains("`from`, `to` and `amount` share"));
        assert!(finding
            .message
            .contains("`amount: u64` looks like a domain value"));
        assert!(finding.message.contains("all 3 parameters"));
        assert!(finding
            .suggestion
            .contains("`Amount` for parameter `amount: u64`"));
        assert!(finding.suggestion.contains("`from` and `to`"));
    }

    #[test]
    fn only_plain_pub_functions_are_checked() {
        for vis in [
            "pub(crate)",
            "pub(super)",
            "pub(in crate::a)",
            "pub(self)",
            "",
        ] {
            let src = format!("{vis} fn f(a: u64, b: u64, user_id: u64) {{}}");
            assert!(!flagged(&src), "`{vis}` should be skipped");
        }
        assert!(flagged("pub fn f(a: u64, b: u64) {}"));
    }

    #[test]
    fn methods_are_checked_and_the_receiver_is_not_a_parameter() {
        assert!(flagged(
            "struct S; impl S { pub fn f(&self, a: u8, b: u16, c: u32) {} }"
        ));
        assert!(!flagged(
            "struct S; impl S { pub fn f(&self, a: u8, b: u16) {} }"
        ));
        assert!(flagged(
            "struct S; impl S { pub fn f(&mut self, a: u64, b: u64) {} }"
        ));
        assert!(flagged(
            "struct S; impl S { pub fn new(user_id: u64) -> Self { S } }"
        ));
    }

    #[test]
    fn trait_impl_methods_are_skipped() {
        let mut sigs = extract_functions(
            "impl Foo for Bar { fn f(a: u64, b: u64, user_id: u64) {} }",
            "src/lib.rs",
        )
        .unwrap();
        let sig = &mut sigs[0];
        assert_eq!(sig.trait_impl.as_deref(), Some("Foo"));
        // Even if some future extractor reports such a method as `pub`, it stays exempt.
        sig.visibility = Visibility::Pub;
        assert!(PrimitiveHeavySignature
            .check(sig, &LintConfig::default())
            .is_empty());
        sig.trait_impl = None;
        assert_eq!(
            PrimitiveHeavySignature
                .check(sig, &LintConfig::default())
                .len(),
            1
        );
    }

    #[test]
    fn the_opt_out_comment_is_honoured() {
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(PrimitiveHeavySignature)];
        let cfg = LintConfig::default();

        let plain = extract_functions("pub fn f(a: u64, b: u64) {}", "src/lib.rs").unwrap();
        assert_eq!(lint_function(&rules, &plain[0], &cfg).len(), 1);

        for comment in [
            "// codesearch-lint: allow(R001)",
            "// codesearch-lint: allow(R002, R001)",
            "//codesearch-lint:allow( r001 )",
        ] {
            let src = format!("{comment}\npub fn f(a: u64, b: u64) {{}}");
            let sigs = extract_functions(&src, "src/lib.rs").unwrap();
            assert!(
                lint_function(&rules, &sigs[0], &cfg).is_empty(),
                "{comment}"
            );
        }

        let other = extract_functions(
            "// codesearch-lint: allow(R002)\npub fn f(a: u64, b: u64) {}",
            "src/lib.rs",
        )
        .unwrap();
        assert_eq!(lint_function(&rules, &other[0], &cfg).len(), 1);
    }

    #[test]
    fn domain_words_are_configurable() {
        let custom = LintConfig {
            domain_words: vec!["sku".to_string()],
            ..LintConfig::default()
        };
        assert_eq!(check_all("pub fn f(sku: String) {}", &custom).len(), 1);
        assert!(check_all("pub fn f(email: String) {}", &custom).is_empty());
    }

    #[test]
    fn known_false_positive_predicates_with_domain_names_are_flagged() {
        // Documents current behaviour: the rule text counts `bool` as a bare primitive.
        assert!(flagged(
            "pub fn is_valid_user(user: &User) -> bool { true }"
        ));
    }
}
