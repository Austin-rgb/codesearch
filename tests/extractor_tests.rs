use codesearch::extractor::{extract_functions, module_path_for_file};
use codesearch::types::{FnKind, FunctionSig, TypeExpr};

fn extract(src: &str) -> Vec<FunctionSig> {
    extract_functions(src, "src/lib.rs").expect("test source should parse")
}

fn find<'a>(sigs: &'a [FunctionSig], name: &str) -> &'a FunctionSig {
    sigs.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no function named `{name}`"))
}

#[test]
fn extracts_name_params_and_return_type() {
    let sigs = extract(
        "
        fn login(username: Username, password: Password) -> Session { todo!() }
        ",
    );
    assert_eq!(sigs.len(), 1);
    let f = &sigs[0];
    assert_eq!(f.name, "login");
    assert_eq!(f.kind, FnKind::Function);
    assert!(!f.has_self);
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.params[0].name, "username");
    assert_eq!(f.params[0].ty, TypeExpr::simple("Username"));
    assert_eq!(f.params[1].name, "password");
    assert_eq!(f.ret, TypeExpr::simple("Session"));
    assert_eq!(f.signature_string(), "(Username, Password) -> Session");
}

#[test]
fn missing_return_type_is_unit() {
    let sigs = extract("fn run(id: UserId) {}");
    assert_eq!(sigs[0].ret, TypeExpr::Unit);
    assert_eq!(sigs[0].signature_string(), "UserId -> ()");
}

#[test]
fn line_numbers_point_at_the_function_name() {
    let src = "\
fn first() {}

/// docs
#[inline]
pub fn second(
    a: u32,
) -> u32 {
    a
}

struct S;
impl S {
    fn method(&self) {}
}
";
    let sigs = extract(src);
    assert_eq!(find(&sigs, "first").line, 1);
    assert_eq!(find(&sigs, "second").line, 5);
    assert_eq!(find(&sigs, "method").line, 13);
}

#[test]
fn methods_record_their_impl_and_receiver() {
    let sigs = extract(
        "
        struct Repo;
        impl Repo {
            fn get(&self, id: UserId) -> User { todo!() }
            fn set(&mut self, id: UserId) {}
            fn take(self) -> Repo { todo!() }
            fn new() -> Self { todo!() }
        }
        ",
    );

    let get = find(&sigs, "get");
    assert_eq!(get.kind, FnKind::Method);
    assert!(get.has_self);
    assert_eq!(get.impl_target.as_deref(), Some("Repo"));
    assert_eq!(get.params[0].name, "self");
    assert_eq!(
        get.params[0].ty,
        TypeExpr::Ref {
            mutable: false,
            inner: Box::new(TypeExpr::simple("Self"))
        }
    );
    assert_eq!(get.signature_string(), "(&Self, UserId) -> User");
    assert_eq!(get.kind_label(), "method of Repo");

    assert_eq!(
        find(&sigs, "set").params[0].ty,
        TypeExpr::Ref {
            mutable: true,
            inner: Box::new(TypeExpr::simple("Self"))
        }
    );
    assert_eq!(find(&sigs, "take").params[0].ty, TypeExpr::simple("Self"));

    let new = find(&sigs, "new");
    assert_eq!(new.kind, FnKind::Method);
    assert!(!new.has_self);
    assert_eq!(new.kind_label(), "associated function of Repo");
}

#[test]
fn trait_impls_and_trait_declarations_are_labelled() {
    let sigs = extract(
        "
        trait Store { fn get(&self, id: UserId) -> User; }
        impl Store for Memory { fn get(&self, id: UserId) -> User { todo!() } }
        impl<T: Clone> Wrapper<T> { fn inner(&self) -> T { todo!() } }
        ",
    );
    let labels: Vec<_> = sigs.iter().map(|s| s.impl_target.clone().unwrap()).collect();
    assert_eq!(labels, vec!["trait Store", "Store for Memory", "Wrapper<T>"]);
}

#[test]
fn generics_are_recorded_by_name() {
    let sigs = extract("fn f<'a, T: Clone, const N: usize>(x: &'a [T; N]) -> T { todo!() }");
    assert_eq!(sigs[0].generics, vec!["'a", "T", "const N"]);
    assert_eq!(sigs[0].signature_string(), "&[T; N] -> T");
}

#[test]
fn nested_functions_are_standalone_functions() {
    let sigs = extract(
        "
        struct S;
        impl S {
            fn outer(&self) {
                fn helper(x: u8) -> u8 { x }
            }
        }
        ",
    );
    let helper = find(&sigs, "helper");
    assert_eq!(helper.kind, FnKind::Function);
    assert_eq!(helper.impl_target, None);
}

#[test]
fn qualified_and_exotic_types_do_not_panic() {
    let sigs = extract(
        "
        fn a(f: fn(u32) -> u32, p: *const u8) -> ! { loop {} }
        fn b(x: <T as Trait>::Output) {}
        fn c(f: impl Fn(A, B) -> C) {}
        fn d(it: impl Iterator<Item = u32>) {}
        ",
    );
    assert_eq!(sigs.len(), 4);
    assert_eq!(find(&sigs, "c").signature_string(), "impl Fn<(A, B), C> -> ()");
    assert_eq!(
        find(&sigs, "d").signature_string(),
        "impl Iterator<Item = u32> -> ()"
    );
}

#[test]
fn module_paths_come_from_the_file_path_and_inline_modules() {
    assert_eq!(module_path_for_file("src/auth/login.rs"), "auth::login");
    assert_eq!(module_path_for_file("src/auth/mod.rs"), "auth");
    assert_eq!(module_path_for_file("src/lib.rs"), "");
    assert_eq!(module_path_for_file("src/main.rs"), "");
    assert_eq!(module_path_for_file("crates/core/src/x/y.rs"), "x::y");
    assert_eq!(module_path_for_file("tests/integration.rs"), "tests::integration");
    assert_eq!(module_path_for_file("build.rs"), "build");

    let sigs = extract_functions(
        "
        fn top() {}
        mod inner {
            pub fn deep() {}
            mod innermost { pub fn deepest() {} }
        }
        ",
        "src/auth/login.rs",
    )
    .unwrap();
    assert_eq!(find(&sigs, "top").module, "auth::login");
    assert_eq!(find(&sigs, "deep").module, "auth::login::inner");
    assert_eq!(find(&sigs, "deepest").module, "auth::login::inner::innermost");
}

#[test]
fn invalid_rust_is_an_error_not_a_panic() {
    assert!(extract_functions("fn broken( {", "src/x.rs").is_err());
}

#[test]
fn files_without_functions_yield_nothing() {
    assert!(extract("struct A; const B: u8 = 1;").is_empty());
}
