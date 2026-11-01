//! Type-expression AST shared by the query parser, the extractor and the matcher.
//!
//! Types are compared *syntactically*: `Session` and `auth::Session` are the same
//! name for matching purposes, but `Session` and a type alias that happens to
//! resolve to it are not (alias resolution is an explicit non-goal of the prototype).

use serde::{Deserialize, Serialize};
use std::fmt;

/// A syntactic Rust type expression (or a query wildcard).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeExpr {
    /// `*` in a query. Matches any single type expression. Never produced by the extractor.
    Wildcard,
    /// `()`.
    Unit,
    /// A named type such as `Session`, `auth::Session` or `Option<UserDetails>`.
    /// Generic arguments live on the last segment.
    Path {
        segments: Vec<String>,
        args: Vec<TypeExpr>,
    },
    /// `(A, B)`. Never empty (the empty tuple is [`TypeExpr::Unit`]).
    Tuple(Vec<TypeExpr>),
    /// `&T` / `&mut T`. Lifetimes are ignored.
    Ref { mutable: bool, inner: Box<TypeExpr> },
    /// `[T]`.
    Slice(Box<TypeExpr>),
    /// `[T; N]`. `None` as the length only appears in queries (`[T; *]`) and matches any length.
    Array(Box<TypeExpr>, Option<String>),
    /// `impl A + B` (`dyn_ == false`) or `dyn A + B` (`dyn_ == true`). Lifetime bounds are ignored.
    TraitObject { dyn_: bool, bounds: Vec<TypeExpr> },
    /// Anything the prototype does not model structurally (fn pointers, raw pointers,
    /// qualified paths, `!`, ...). Compared by its token text.
    Other(String),
}

impl TypeExpr {
    /// A bare type name, e.g. `TypeExpr::simple("Session")`. `"a::B"` becomes two segments.
    pub fn simple(name: &str) -> Self {
        Self::generic(name, Vec::new())
    }

    /// A named type with generic arguments, e.g. `Option<UserDetails>`.
    pub fn generic(name: &str, args: Vec<TypeExpr>) -> Self {
        TypeExpr::Path {
            segments: name.split("::").map(str::to_string).collect(),
            args,
        }
    }

    /// A cheap, stable key for the outermost shape of the type, used to pre-filter
    /// candidates in SQL before structural matching. Two types that can match
    /// structurally must have equal heads (or one of them has no head).
    ///
    /// Note: a future alias resolver that rewrites types *before* matching must
    /// not rely on this pre-filter for the types it rewrites.
    pub fn head(&self) -> Option<String> {
        match self {
            TypeExpr::Path { segments, .. } => segments.last().cloned(),
            TypeExpr::Unit => Some("()".to_string()),
            TypeExpr::Tuple(_) => Some("(,)".to_string()),
            _ => None,
        }
    }
}

fn write_list(f: &mut fmt::Formatter<'_>, items: &[TypeExpr]) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeExpr::Wildcard => f.write_str("*"),
            TypeExpr::Unit => f.write_str("()"),
            TypeExpr::Path { segments, args } => {
                f.write_str(&segments.join("::"))?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    write_list(f, args)?;
                    f.write_str(">")?;
                }
                Ok(())
            }
            TypeExpr::Tuple(items) => {
                f.write_str("(")?;
                write_list(f, items)?;
                if items.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            TypeExpr::Ref { mutable, inner } => {
                write!(f, "&{}{}", if *mutable { "mut " } else { "" }, inner)
            }
            TypeExpr::Slice(inner) => write!(f, "[{inner}]"),
            TypeExpr::Array(inner, Some(len)) => write!(f, "[{inner}; {len}]"),
            TypeExpr::Array(inner, None) => write!(f, "[{inner}; *]"),
            TypeExpr::TraitObject { dyn_, bounds } => {
                f.write_str(if *dyn_ { "dyn " } else { "impl " })?;
                for (i, bound) in bounds.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" + ")?;
                    }
                    write!(f, "{bound}")?;
                }
                Ok(())
            }
            TypeExpr::Other(text) => f.write_str(text),
        }
    }
}

/// A parsed signature query: `(params...) -> ret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub params: Vec<TypeExpr>,
    pub ret: TypeExpr,
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params: Vec<&TypeExpr> = self.params.iter().collect();
        f.write_str(&format_signature(&params, &self.ret))
    }
}

/// Renders `(A, B) -> R`. A single parameter that is not a tuple/unit is written
/// without parentheses (`A -> R`), mirroring the query syntax.
pub fn format_signature(params: &[&TypeExpr], ret: &TypeExpr) -> String {
    let lhs = match params {
        [single] if !matches!(**single, TypeExpr::Tuple(_) | TypeExpr::Unit) => single.to_string(),
        _ => {
            let joined: Vec<String> = params.iter().map(|t| t.to_string()).collect();
            format!("({})", joined.join(", "))
        }
    };
    format!("{lhs} -> {ret}")
}

/// Whether a function lives in an `impl`/`trait` block or at module level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnKind {
    Function,
    Method,
}

impl FnKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FnKind::Function => "function",
            FnKind::Method => "method",
        }
    }

    pub fn from_db(s: &str) -> Self {
        if s == "method" {
            FnKind::Method
        } else {
            FnKind::Function
        }
    }
}

/// Nominal visibility of a function as written in the source (`pub`, `pub(crate)`, ...).
///
/// This is purely syntactic: a `pub fn` inside a private module is still `Pub`, and methods
/// in trait impls, which cannot carry a visibility keyword, are `Private`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Pub,
    PubCrate,
    PubSuper,
    /// `pub(in some::path)`; the path is stored without whitespace.
    PubIn(String),
    /// No visibility keyword (also `pub(self)`).
    Private,
}

impl Visibility {
    /// Inverse of the `Display` form, as stored in the index.
    pub fn from_db(s: &str) -> Self {
        match s {
            "pub" => Visibility::Pub,
            "pub(crate)" => Visibility::PubCrate,
            "pub(super)" => Visibility::PubSuper,
            other => match other
                .strip_prefix("pub(in ")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                Some(path) => Visibility::PubIn(path.to_string()),
                None => Visibility::Private,
            },
        }
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Visibility::Pub => f.write_str("pub"),
            Visibility::PubCrate => f.write_str("pub(crate)"),
            Visibility::PubSuper => f.write_str("pub(super)"),
            Visibility::PubIn(path) => write!(f, "pub(in {path})"),
            Visibility::Private => f.write_str("private"),
        }
    }
}

/// One named, typed parameter. For receivers the name is `self` and the type is
/// `Self`, `&Self` or `&mut Self` (or the explicit type for `self: Box<Self>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
}

/// Everything the index stores about one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSig {
    pub name: String,
    /// Module path derived from the file path plus inline `mod` blocks, e.g. `auth::login`.
    /// Empty at the crate root.
    pub module: String,
    pub kind: FnKind,
    /// For methods: `Repo`, `Display for Repo`, or `trait Store`.
    pub impl_target: Option<String>,
    /// True when the first parameter is a `self` receiver.
    pub has_self: bool,
    /// Generic parameter names, e.g. `["'a", "T"]`.
    pub generics: Vec<String>,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
    /// Path relative to the project root, always with `/` separators.
    pub file: String,
    pub line: usize,
    pub visibility: Visibility,
    /// Full doc-comment text (one `\n`-separated string, one leading space stripped per
    /// line), or `None` when the function has no doc comment.
    pub doc: Option<String>,
    /// First line of `doc`, trimmed. `Some("")` when the doc starts with a blank line.
    pub doc_first_line: Option<String>,
    /// For methods inside `impl Trait for Type`: the rendered trait path, e.g. `fmt::Display`.
    /// `None` for inherent impls, trait declarations and free functions.
    pub trait_impl: Option<String>,
    /// Rule ids named by a `// codesearch-lint: allow(...)` comment above the function,
    /// upper-cased.
    pub lint_allow: Vec<String>,
}

impl FunctionSig {
    pub fn signature_string(&self) -> String {
        let params: Vec<&TypeExpr> = self.params.iter().map(|p| &p.ty).collect();
        format_signature(&params, &self.ret)
    }

    /// Human-readable description of where the function lives, for result output.
    pub fn kind_label(&self) -> String {
        match (self.kind, &self.impl_target) {
            (FnKind::Method, Some(target)) if self.has_self => format!("method of {target}"),
            (FnKind::Method, Some(target)) => format!("associated function of {target}"),
            (FnKind::Method, None) => "method".to_string(),
            (FnKind::Function, _) => "function".to_string(),
        }
    }
}
