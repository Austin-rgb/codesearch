//! Matching tests. Each test feeds real Rust source through the extractor and a real
//! query through the parser, so the whole parse → extract → match pipeline is exercised.

use codesearch::extractor::extract_functions;
use codesearch::matcher::{type_matches, MatchOptions, Matcher};
use codesearch::parser::parse_query;
use codesearch::types::TypeExpr;

fn names_with(src: &str, query: &str, opts: MatchOptions) -> Vec<String> {
    let sigs = extract_functions(src, "src/lib.rs").expect("test source should parse");
    let query = parse_query(query).expect("test query should parse");
    let matcher = Matcher::default();
    sigs.iter()
        .filter(|sig| matcher.matches(&query, sig, opts))
        .map(|sig| sig.name.clone())
        .collect()
}

fn names(src: &str, query: &str) -> Vec<String> {
    names_with(src, query, MatchOptions::default())
}

fn ignoring_self() -> MatchOptions {
    MatchOptions { ignore_self: true }
}

#[test]
fn single_parameter() {
    let src = "
        fn get(id: UserId) -> UserDetails { todo!() }
        fn other(id: UserId) -> User { todo!() }
        fn wrong_input(id: TenantId) -> UserDetails { todo!() }
    ";
    assert_eq!(names(src, "UserId -> UserDetails"), vec!["get"]);
}

#[test]
fn multiple_parameters() {
    let src = "
        fn login(username: Username, password: Password) -> Session { todo!() }
        fn logout(session: Session) {}
    ";
    assert_eq!(names(src, "(Username, Password) -> Session"), vec!["login"]);
}

#[test]
fn parameter_order_matters() {
    let src = "
        fn login(username: Username, password: Password) -> Session { todo!() }
        fn login_swapped(password: Password, username: Username) -> Session { todo!() }
    ";
    assert_eq!(names(src, "(Username, Password) -> Session"), vec!["login"]);
    assert_eq!(
        names(src, "(Password, Username) -> Session"),
        vec!["login_swapped"]
    );
}

#[test]
fn parameter_count_must_match() {
    let src = "
        fn two(a: A, b: B) -> C { todo!() }
        fn three(a: A, b: B, c: B) -> C { todo!() }
    ";
    assert_eq!(names(src, "(A, B) -> C"), vec!["two"]);
    assert_eq!(names(src, "(A, B, B) -> C"), vec!["three"]);
    assert!(names(src, "A -> C").is_empty());
}

#[test]
fn empty_parameter_list() {
    let src = "
        fn load() -> Config { todo!() }
        fn load_from(path: Path) -> Config { todo!() }
    ";
    assert_eq!(names(src, "() -> Config"), vec!["load"]);
    assert_eq!(names(src, "Path -> Config"), vec!["load_from"]);
}

#[test]
fn unit_return_types_explicit_and_implicit() {
    let src = "
        fn implicit(id: UserId) {}
        fn explicit(id: UserId) -> () {}
        fn returns_value(id: UserId) -> Session { todo!() }
    ";
    assert_eq!(names(src, "UserId -> ()"), vec!["implicit", "explicit"]);
    assert_eq!(names(src, "UserId -> Session"), vec!["returns_value"]);
}

#[test]
fn tuple_return_types() {
    let src = "
        fn both(u: UserId, t: TenantId) -> (User, Metadata) { todo!() }
        fn reversed(u: UserId, t: TenantId) -> (Metadata, User) { todo!() }
        fn triple(u: UserId, t: TenantId) -> (User, Metadata, Extra) { todo!() }
        fn plain(u: UserId, t: TenantId) -> User { todo!() }
    ";
    assert_eq!(
        names(src, "(UserId, TenantId) -> (User, Metadata)"),
        vec!["both"]
    );
    assert_eq!(
        names(src, "(UserId, TenantId) -> (Metadata, User)"),
        vec!["reversed"]
    );
    assert_eq!(
        names(src, "(UserId, TenantId) -> (User, Metadata, Extra)"),
        vec!["triple"]
    );
}

#[test]
fn tuple_parameters_are_distinct_from_two_parameters() {
    let src = "
        fn takes_tuple(pair: (A, B)) -> C { todo!() }
        fn takes_two(a: A, b: B) -> C { todo!() }
    ";
    assert_eq!(names(src, "((A, B)) -> C"), vec!["takes_tuple"]);
    assert_eq!(names(src, "(A, B) -> C"), vec!["takes_two"]);
}

#[test]
fn generic_return_types() {
    let src = "
        fn get_user(id: UserId) -> Option<UserDetails> { todo!() }
        fn find_user(id: UserId) -> Result<UserDetails, Error> { todo!() }
    ";
    assert_eq!(
        names(src, "UserId -> Option<UserDetails>"),
        vec!["get_user"]
    );
    assert_eq!(
        names(src, "UserId -> Result<UserDetails, Error>"),
        vec!["find_user"]
    );
    // Option<..> never matches Result<..>, and argument counts must agree.
    assert!(names(src, "UserId -> Result<UserDetails>").is_empty());
    assert!(names(src, "UserId -> Option<User>").is_empty());
}

#[test]
fn nested_generic_types() {
    let src = "
        fn deep(x: Vec<Option<Result<A, B>>>) -> HashMap<String, Vec<u8>> { todo!() }
        fn shallow(x: Vec<Option<A>>) -> HashMap<String, Vec<u8>> { todo!() }
    ";
    assert_eq!(
        names(src, "Vec<Option<Result<A, B>>> -> HashMap<String, Vec<u8>>"),
        vec!["deep"]
    );
    assert_eq!(
        names(src, "Vec<Option<Result<A, C>>> -> HashMap<String, Vec<u8>>"),
        Vec::<String>::new()
    );
    assert_eq!(
        names(src, "Vec<Option<*>> -> HashMap<String, Vec<u8>>"),
        vec!["deep", "shallow"]
    );
}

#[test]
fn reference_types() {
    let src = "
        fn f(name: &str, buf: &mut Vec<u8>) -> &str { todo!() }
    ";
    assert_eq!(names(src, "(&str, &mut Vec<u8>) -> &str"), vec!["f"]);
    // `str` is not `&str`, and `&Vec` is not `&mut Vec`.
    assert!(names(src, "(str, &mut Vec<u8>) -> &str").is_empty());
    assert!(names(src, "(&str, &Vec<u8>) -> &str").is_empty());
    assert!(names(src, "(&str, &mut Vec<u8>) -> str").is_empty());
}

#[test]
fn lifetimes_do_not_affect_matching() {
    let src = "
        fn first<'a>(s: &'a str) -> &'a str { todo!() }
        fn with_cow<'a>(s: Cow<'a, str>) -> Cow<'a, str> { todo!() }
    ";
    assert_eq!(names(src, "&str -> &str"), vec!["first"]);
    assert_eq!(names(src, "&'a str -> &'a str"), vec!["first"]);
    assert_eq!(names(src, "Cow<str> -> Cow<str>"), vec!["with_cow"]);
}

#[test]
fn wildcard_parameter() {
    let src = "
        fn a(u: Username, p: Password) -> Session { todo!() }
        fn b(u: Username, n: Option<Vec<String>>) -> Session { todo!() }
        fn c(p: Password, u: Username) -> Session { todo!() }
        fn d(u: Username, p: Password) -> Other { todo!() }
    ";
    assert_eq!(names(src, "(Username, *) -> Session"), vec!["a", "b"]);
    assert_eq!(names(src, "(*, Username) -> Session"), vec!["c"]);
}

#[test]
fn wildcard_return_type() {
    let src = "
        fn a(u: Username, p: Password) -> Session { todo!() }
        fn b(u: Username, p: Password) {}
        fn c(u: Username, p: Password) -> Result<Session, Error> { todo!() }
        fn d(u: Username) -> Session { todo!() }
    ";
    assert_eq!(names(src, "(Username, Password) -> *"), vec!["a", "b", "c"]);
}

#[test]
fn wildcard_matches_exactly_one_type_not_zero_or_many() {
    let src = "
        fn zero() -> A { todo!() }
        fn one(x: X) -> A { todo!() }
        fn two(x: X, y: Y) -> A { todo!() }
    ";
    assert_eq!(names(src, "* -> A"), vec!["one"]);
    assert_eq!(names(src, "(*, *) -> A"), vec!["two"]);
    assert_eq!(names(src, "() -> A"), vec!["zero"]);
}

#[test]
fn wildcard_inside_generics() {
    let src = "
        fn a(x: Vec<String>) -> Option<u32> { todo!() }
        fn b(x: Vec<(A, B)>) -> Option<u32> { todo!() }
        fn c(x: HashSet<String>) -> Option<u32> { todo!() }
    ";
    assert_eq!(names(src, "Vec<*> -> Option<*>"), vec!["a", "b"]);
}

#[test]
fn methods_with_self_count_the_receiver_as_the_first_parameter() {
    let src = "
        struct Repo;
        impl Repo {
            fn get(&self, id: UserId) -> User { todo!() }
            fn set(&mut self, id: UserId) {}
            fn create(name: Username) -> Self { todo!() }
            fn into_inner(self) -> Repo { todo!() }
        }
    ";
    assert_eq!(names(src, "(&Self, UserId) -> User"), vec!["get"]);
    assert_eq!(names(src, "(&mut Self, UserId) -> ()"), vec!["set"]);
    assert_eq!(names(src, "Username -> Self"), vec!["create"]);
    assert_eq!(names(src, "Self -> Repo"), vec!["into_inner"]);
    // without --ignore-self the receiver is a real parameter
    assert!(names(src, "UserId -> User").is_empty());
}

#[test]
fn ignore_self_hides_the_receiver() {
    let src = "
        struct Repo;
        impl Repo {
            fn get(&self, id: UserId) -> User { todo!() }
            fn set(&mut self, id: UserId) {}
            fn create(name: Username) -> Self { todo!() }
            fn into_inner(self) -> Repo { todo!() }
        }
        fn free(id: UserId) -> User { todo!() }
    ";
    assert_eq!(
        names_with(src, "UserId -> User", ignoring_self()),
        vec!["get", "free"]
    );
    assert_eq!(
        names_with(src, "UserId -> ()", ignoring_self()),
        vec!["set"]
    );
    assert_eq!(
        names_with(src, "() -> Repo", ignoring_self()),
        vec!["into_inner"]
    );
    // functions without a receiver are unaffected
    assert_eq!(
        names_with(src, "Username -> Self", ignoring_self()),
        vec!["create"]
    );
}

#[test]
fn explicitly_typed_receivers() {
    let src = "
        struct Node;
        impl Node {
            fn consume(self: Box<Self>) -> Node { todo!() }
            fn shared(self: &Self) -> u32 { todo!() }
        }
    ";
    assert_eq!(names(src, "Box<Self> -> Node"), vec!["consume"]);
    assert_eq!(names(src, "&Self -> u32"), vec!["shared"]);
}

#[test]
fn generic_functions_match_on_their_written_type_names() {
    let src = "
        fn largest<T: PartialOrd>(items: &[T]) -> T { todo!() }
        fn wrap<T>(v: T) -> Option<T> { todo!() }
        fn convert<T, U>(t: T) -> U where T: Into<U> { todo!() }
        fn const_len<const N: usize>(a: [u8; N]) -> usize { N }
    ";
    assert_eq!(names(src, "&[T] -> T"), vec!["largest"]);
    assert_eq!(names(src, "T -> Option<T>"), vec!["wrap"]);
    assert_eq!(names(src, "T -> U"), vec!["convert"]);
    assert_eq!(names(src, "[u8; N] -> usize"), vec!["const_len"]);
    assert_eq!(names(src, "[u8; *] -> usize"), vec!["const_len"]);
    assert!(names(src, "[u8; 4] -> usize").is_empty());
}

#[test]
fn impl_trait_and_dyn_trait() {
    let src = "
        fn show(x: impl Display) -> String { todo!() }
        fn boxed(x: Box<dyn Error>) {}
        fn both(x: impl Read + Write) {}
    ";
    assert_eq!(names(src, "impl Display -> String"), vec!["show"]);
    assert_eq!(names(src, "Box<dyn Error> -> ()"), vec!["boxed"]);
    assert_eq!(names(src, "impl Read + Write -> ()"), vec!["both"]);
    // `impl` and `dyn` are different things
    assert!(names(src, "Box<impl Error> -> ()").is_empty());
}

#[test]
fn path_prefixes_are_matched_as_suffixes() {
    let src = "
        fn a(m: std::collections::HashMap<K, V>) {}
        fn b(m: HashMap<K, V>) {}
        fn c(m: other::HashMap<K, V>) {}
    ";
    assert_eq!(names(src, "HashMap<K, V> -> ()"), vec!["a", "b", "c"]);
    // an unqualified candidate (`b`) is compatible with any qualified query
    assert_eq!(
        names(src, "collections::HashMap<K, V> -> ()"),
        vec!["a", "b"]
    );
    assert_eq!(
        names(src, "std::collections::HashMap<K, V> -> ()"),
        vec!["a", "b"]
    );
    assert_eq!(names(src, "other::HashMap<K, V> -> ()"), vec!["b", "c"]);
}

#[test]
fn async_and_const_functions_are_plain_signatures() {
    let src = "
        async fn fetch(id: UserId) -> User { todo!() }
        const fn zero() -> u32 { 0 }
    ";
    assert_eq!(names(src, "UserId -> User"), vec!["fetch"]);
    assert_eq!(names(src, "() -> u32"), vec!["zero"]);
}

#[test]
fn trait_methods_are_searchable() {
    let src = "
        trait Store {
            fn get(&self, id: UserId) -> Option<User>;
            fn default_user() -> User { todo!() }
        }
        impl Store for Memory {
            fn get(&self, id: UserId) -> Option<User> { todo!() }
            fn default_user() -> User { todo!() }
        }
    ";
    assert_eq!(
        names(src, "() -> User"),
        vec!["default_user", "default_user"]
    );
    assert_eq!(
        names_with(src, "UserId -> Option<User>", ignoring_self()),
        vec!["get", "get"]
    );
}

#[test]
fn type_matches_directly() {
    let session = TypeExpr::simple("Session");
    let option_session = TypeExpr::generic("Option", vec![session.clone()]);

    assert!(type_matches(&TypeExpr::Wildcard, &session));
    assert!(type_matches(&TypeExpr::Wildcard, &option_session));
    assert!(type_matches(&TypeExpr::Wildcard, &TypeExpr::Unit));
    assert!(type_matches(&session, &session));
    assert!(!type_matches(&session, &option_session));
    assert!(!type_matches(&option_session, &session));
    assert!(!type_matches(&TypeExpr::Unit, &session));
    assert!(!type_matches(
        &TypeExpr::Unit,
        &TypeExpr::Tuple(vec![session.clone()])
    ));
    assert!(!type_matches(&session, &TypeExpr::Wildcard));
}
