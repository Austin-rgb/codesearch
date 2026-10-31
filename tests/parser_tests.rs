use codesearch::parser::parse_query;
use codesearch::types::{Query, TypeExpr};

fn t(name: &str) -> TypeExpr {
    TypeExpr::simple(name)
}

fn g(name: &str, args: Vec<TypeExpr>) -> TypeExpr {
    TypeExpr::generic(name, args)
}

fn parse(q: &str) -> Query {
    parse_query(q).unwrap_or_else(|e| panic!("`{q}` should parse, got:\n{e}"))
}

fn parse_err(q: &str) -> String {
    match parse_query(q) {
        Ok(parsed) => panic!("`{q}` should not parse, got {parsed:?}"),
        Err(e) => e.message,
    }
}

#[test]
fn single_parameter_without_parentheses() {
    let q = parse("UserId -> UserDetails");
    assert_eq!(q.params, vec![t("UserId")]);
    assert_eq!(q.ret, t("UserDetails"));
}

#[test]
fn single_parameter_with_parentheses_is_the_same_query() {
    assert_eq!(parse("(UserId) -> UserDetails"), parse("UserId -> UserDetails"));
}

#[test]
fn multiple_parameters_keep_their_order() {
    let q = parse("(Username, Password) -> Session");
    assert_eq!(q.params, vec![t("Username"), t("Password")]);
    assert_eq!(q.ret, t("Session"));

    let swapped = parse("(Password, Username) -> Session");
    assert_ne!(q, swapped);
}

#[test]
fn empty_parameter_list() {
    let q = parse("() -> Config");
    assert!(q.params.is_empty());
    assert_eq!(q.ret, t("Config"));
}

#[test]
fn unit_return_type() {
    let q = parse("UserId -> ()");
    assert_eq!(q.params, vec![t("UserId")]);
    assert_eq!(q.ret, TypeExpr::Unit);
}

#[test]
fn unit_to_unit() {
    let q = parse("() -> ()");
    assert!(q.params.is_empty());
    assert_eq!(q.ret, TypeExpr::Unit);
}

#[test]
fn tuple_return_type() {
    let q = parse("(UserId, TenantId) -> (User, Metadata)");
    assert_eq!(q.params, vec![t("UserId"), t("TenantId")]);
    assert_eq!(q.ret, TypeExpr::Tuple(vec![t("User"), t("Metadata")]));
}

#[test]
fn parentheses_around_a_single_return_type_are_optional() {
    assert_eq!(parse("UserId -> (User)"), parse("UserId -> User"));
}

#[test]
fn single_element_tuple_needs_a_trailing_comma() {
    let q = parse("UserId -> (User,)");
    assert_eq!(q.ret, TypeExpr::Tuple(vec![t("User")]));
}

#[test]
fn single_tuple_parameter_needs_double_parentheses() {
    let q = parse("((A, B)) -> C");
    assert_eq!(q.params, vec![TypeExpr::Tuple(vec![t("A"), t("B")])]);
}

#[test]
fn generic_return_type() {
    let q = parse("UserId -> Option<UserDetails>");
    assert_eq!(q.ret, g("Option", vec![t("UserDetails")]));
}

#[test]
fn generic_with_two_arguments() {
    let q = parse("(Username, Password) -> Result<Session, AuthError>");
    assert_eq!(q.ret, g("Result", vec![t("Session"), t("AuthError")]));
}

#[test]
fn deeply_nested_generics_with_adjacent_closing_brackets() {
    let q = parse("Vec<Option<Result<A, B>>> -> HashMap<String, Vec<u8>>");
    assert_eq!(
        q.params,
        vec![g(
            "Vec",
            vec![g("Option", vec![g("Result", vec![t("A"), t("B")])])]
        )]
    );
    assert_eq!(q.ret, g("HashMap", vec![t("String"), g("Vec", vec![t("u8")])]));
}

#[test]
fn generic_arguments_may_have_a_trailing_comma() {
    assert_eq!(parse("A -> Result<B, C,>"), parse("A -> Result<B, C>"));
}

#[test]
fn reference_types() {
    let q = parse("(&str, &mut Vec<u8>) -> &User");
    assert_eq!(
        q.params,
        vec![
            TypeExpr::Ref {
                mutable: false,
                inner: Box::new(t("str"))
            },
            TypeExpr::Ref {
                mutable: true,
                inner: Box::new(g("Vec", vec![t("u8")]))
            },
        ]
    );
    assert_eq!(
        q.ret,
        TypeExpr::Ref {
            mutable: false,
            inner: Box::new(t("User"))
        }
    );
}

#[test]
fn lifetimes_are_accepted_and_ignored() {
    assert_eq!(parse("&'a str -> &'a mut User"), parse("&str -> &mut User"));
    assert_eq!(parse("Cow<'a, str> -> ()"), parse("Cow<str> -> ()"));
}

#[test]
fn slices_and_arrays() {
    let q = parse("(&[u8], [u32; 4], [u32; N], [u32; *]) -> ()");
    assert_eq!(q.params.len(), 4);
    assert_eq!(
        q.params[1],
        TypeExpr::Array(Box::new(t("u32")), Some("4".to_string()))
    );
    assert_eq!(
        q.params[2],
        TypeExpr::Array(Box::new(t("u32")), Some("N".to_string()))
    );
    assert_eq!(q.params[3], TypeExpr::Array(Box::new(t("u32")), None));
}

#[test]
fn impl_and_dyn_trait_types() {
    let q = parse("impl Display -> Box<dyn Error>");
    assert_eq!(
        q.params,
        vec![TypeExpr::TraitObject {
            dyn_: false,
            bounds: vec![t("Display")]
        }]
    );
    assert_eq!(
        q.ret,
        g(
            "Box",
            vec![TypeExpr::TraitObject {
                dyn_: true,
                bounds: vec![t("Error")]
            }]
        )
    );
}

#[test]
fn trait_object_with_several_bounds() {
    let q = parse("Box<dyn Error + Send + Sync> -> ()");
    assert_eq!(
        q.params,
        vec![g(
            "Box",
            vec![TypeExpr::TraitObject {
                dyn_: true,
                bounds: vec![t("Error"), t("Send"), t("Sync")]
            }]
        )]
    );
}

#[test]
fn paths_with_segments() {
    let q = parse("std::collections::HashMap<K, V> -> ()");
    assert_eq!(
        q.params,
        vec![g("std::collections::HashMap", vec![t("K"), t("V")])]
    );
}

#[test]
fn wildcards() {
    let q = parse("(Username, *) -> Session");
    assert_eq!(q.params, vec![t("Username"), TypeExpr::Wildcard]);

    let q = parse("(Username, Password) -> *");
    assert_eq!(q.ret, TypeExpr::Wildcard);

    let q = parse("Vec<*> -> Option<*>");
    assert_eq!(q.params, vec![g("Vec", vec![TypeExpr::Wildcard])]);
    assert_eq!(q.ret, g("Option", vec![TypeExpr::Wildcard]));

    let q = parse("* -> *");
    assert_eq!(q.params, vec![TypeExpr::Wildcard]);
}

#[test]
fn whitespace_is_insignificant() {
    assert_eq!(
        parse("  (  Username ,Password )->  Session  "),
        parse("(Username, Password) -> Session")
    );
}

#[test]
fn trailing_comma_in_parameter_list() {
    assert_eq!(parse("(A, B,) -> C"), parse("(A, B) -> C"));
}

#[test]
fn display_round_trips_through_the_parser() {
    for q in [
        "(Username, Password) -> Session",
        "UserId -> Option<UserDetails>",
        "() -> Config",
        "UserId -> ()",
        "(&str, &mut Vec<u8>) -> Result<(A, B), std::io::Error>",
        "((A, B)) -> C",
        "(Username, *) -> *",
    ] {
        let parsed = parse(q);
        assert_eq!(parse(&parsed.to_string()), parsed, "round trip of `{q}`");
    }
}

// ------------------------------------------------------------- invalid syntax

#[test]
fn empty_query_is_rejected() {
    assert!(parse_err("").contains("empty"));
    assert!(parse_err("   ").contains("empty"));
}

#[test]
fn missing_arrow_is_rejected() {
    assert!(parse_err("(Username, Password)").contains("expected `->`"));
    assert!(parse_err("Username").contains("expected `->`"));
}

#[test]
fn multiple_parameters_without_parentheses_are_rejected() {
    let msg = parse_err("Username, Password -> Session");
    assert!(msg.contains("parentheses"), "{msg}");
}

#[test]
fn unbalanced_parentheses_are_rejected() {
    assert!(parse_err("(Username, Password -> Session").contains("expected `,` or `)`"));
    assert!(parse_err("(A, B) -> (C, D").contains("`)`"));
}

#[test]
fn unbalanced_angle_brackets_are_rejected() {
    assert!(parse_err("UserId -> Option<UserDetails").contains("expected `>`"));
    assert!(parse_err("Option<UserDetails>> -> A").contains("expected `->`"));
}

#[test]
fn missing_types_are_rejected() {
    assert!(parse_err("UserId ->").contains("expected a type"));
    assert!(parse_err("-> Session").contains("expected a type"));
    assert!(parse_err("(A,, B) -> C").contains("expected a type"));
    assert!(parse_err("A -> Option<>").contains("expected a type"));
}

#[test]
fn trailing_garbage_is_rejected() {
    assert!(parse_err("A -> B C").contains("after the return type"));
    assert!(parse_err("A -> B -> C").contains("after the return type"));
}

#[test]
fn stray_characters_are_rejected() {
    assert!(parse_err("A => B").contains("unexpected character `=`"));
    assert!(parse_err("A - B").contains("`->`"));
    assert!(parse_err("A -> B: C").contains("`::`"));
    assert!(parse_err("A -> 'a").contains("expected a type"));
}

#[test]
fn errors_point_at_the_problem() {
    // the second comma, at byte offset 3, is where a type was expected
    let err = parse_query("(A,, B) -> C").unwrap_err();
    assert_eq!(err.position, 3);

    let rendered = err.to_string();
    assert!(rendered.contains("invalid query"));
    assert!(rendered.contains("(A,, B) -> C"));
    // two spaces of indent + three columns of input, then the caret
    assert!(rendered.contains("\n     ^"), "{rendered}");
}

#[test]
fn missing_parentheses_error_carries_a_hint() {
    let err = parse_query("A, B -> C").unwrap_err();
    assert!(err.hint.as_deref().unwrap_or("").contains("(A, B) -> C"));
}
