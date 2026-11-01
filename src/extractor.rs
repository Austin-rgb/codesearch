//! Extracts function signatures from Rust source with `syn`.
//!
//! Everything is purely syntactic: macros are not expanded, `cfg` attributes are
//! ignored, type aliases are not resolved, and lifetimes / `where` clauses are dropped.
//!
//! To support another language, write a sibling module that produces the same
//! [`FunctionSig`] values; the query parser, matcher and index do not care where
//! signatures come from.

use crate::types::{FnKind, FunctionSig, Param, TypeExpr, Visibility};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Expr, ExprLit, FnArg, GenericArgument, GenericParam, ImplItemFn, ItemFn, ItemImpl,
    ItemMod, ItemTrait, Lit, Meta, Pat, PathArguments, ReturnType, Token, TraitItemFn, Type,
    TypeParamBound,
};

/// Marker that opts a function out of lint rules, e.g. `// codesearch-lint: allow(R001, R002)`.
const ALLOW_MARKER: &str = "codesearch-lint:";

/// Parses `source` and returns every function and method it declares.
///
/// `rel_path` is the file's path relative to the project root (using `/`); it is
/// stored on each result and used to derive the module path.
pub fn extract_functions(source: &str, rel_path: &str) -> syn::Result<Vec<FunctionSig>> {
    let file = syn::parse_file(source)?;
    let mut collector = Collector {
        file: rel_path,
        lines: source.lines().collect(),
        base_module: module_path_for_file(rel_path),
        inline_mods: Vec::new(),
        scopes: Vec::new(),
        out: Vec::new(),
    };
    collector.visit_file(&file);
    Ok(collector.out)
}

/// Derives a module path from a file path, following the usual Cargo layout:
/// everything after the first `src/` component, `mod.rs` / `lib.rs` / `main.rs`
/// contributing no segment of their own.
///
/// `src/auth/login.rs` → `auth::login`, `src/auth/mod.rs` → `auth`, `src/lib.rs` → ``.
pub fn module_path_for_file(rel_path: &str) -> String {
    let parts: Vec<&str> = rel_path.split('/').collect();
    let start = parts
        .iter()
        .position(|p| *p == "src")
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut modules: Vec<String> = parts[start..].iter().map(|s| s.to_string()).collect();
    if let Some(last) = modules.pop() {
        let stem = last.strip_suffix(".rs").unwrap_or(last.as_str());
        if !matches!(stem, "mod" | "lib" | "main") {
            modules.push(stem.to_string());
        }
    }
    modules.join("::")
}

/// An enclosing `impl` or `trait` block.
struct Scope {
    /// `Repo`, `Display for Repo`, or `trait Store`.
    label: String,
    /// Rendered trait path for `impl Trait for Type` blocks only.
    trait_impl: Option<String>,
}

/// Where a function was found, as opposed to what its signature says.
struct Origin {
    kind: FnKind,
    impl_target: Option<String>,
    trait_impl: Option<String>,
}

impl Origin {
    fn free() -> Self {
        Origin {
            kind: FnKind::Function,
            impl_target: None,
            trait_impl: None,
        }
    }

    fn in_scope(scope: Option<&Scope>) -> Self {
        Origin {
            kind: FnKind::Method,
            impl_target: scope.map(|s| s.label.clone()),
            trait_impl: scope.and_then(|s| s.trait_impl.clone()),
        }
    }
}

struct Collector<'a> {
    file: &'a str,
    /// Source lines, for finding `// codesearch-lint:` comments that `syn` discards.
    lines: Vec<&'a str>,
    base_module: String,
    inline_mods: Vec<String>,
    /// Innermost `impl` / `trait` block last.
    scopes: Vec<Scope>,
    out: Vec<FunctionSig>,
}

impl Collector<'_> {
    fn module(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if !self.base_module.is_empty() {
            parts.push(&self.base_module);
        }
        parts.extend(self.inline_mods.iter().map(String::as_str));
        parts.join("::")
    }

    fn record(
        &mut self,
        sig: &syn::Signature,
        vis: Option<&syn::Visibility>,
        attrs: &[Attribute],
        origin: Origin,
    ) {
        let mut params = Vec::new();
        let mut has_self = false;

        for input in &sig.inputs {
            match input {
                FnArg::Receiver(receiver) => {
                    has_self = true;
                    let self_ty = TypeExpr::simple("Self");
                    let ty = if receiver.colon_token.is_some() {
                        // `self: Box<Self>` and friends carry an explicit type.
                        convert_type(&receiver.ty)
                    } else if receiver.reference.is_some() {
                        TypeExpr::Ref {
                            mutable: receiver.mutability.is_some(),
                            inner: Box::new(self_ty),
                        }
                    } else {
                        self_ty
                    };
                    params.push(Param {
                        name: "self".to_string(),
                        ty,
                    });
                }
                FnArg::Typed(typed) => {
                    let name = match &*typed.pat {
                        Pat::Ident(ident) => ident.ident.to_string(),
                        other => other.to_token_stream().to_string(),
                    };
                    params.push(Param {
                        name,
                        ty: convert_type(&typed.ty),
                    });
                }
            }
        }

        let ret = match &sig.output {
            ReturnType::Default => TypeExpr::Unit,
            ReturnType::Type(_, ty) => convert_type(ty),
        };

        let generics = sig
            .generics
            .params
            .iter()
            .map(|param| match param {
                GenericParam::Type(t) => t.ident.to_string(),
                GenericParam::Lifetime(l) => format!("'{}", l.lifetime.ident),
                GenericParam::Const(c) => format!("const {}", c.ident),
            })
            .collect();

        let ident_line = sig.ident.span().start().line;
        let start_line = item_start_line(sig, vis, attrs).min(ident_line);
        let lint_allow = self.lint_allows(start_line, ident_line);
        let doc = doc_text(attrs);
        let doc_first_line = doc
            .as_deref()
            .map(|d| d.split('\n').next().unwrap_or("").trim().to_string());

        self.out.push(FunctionSig {
            name: sig.ident.to_string(),
            module: self.module(),
            kind: origin.kind,
            impl_target: origin.impl_target,
            has_self,
            generics,
            params,
            ret,
            file: self.file.to_string(),
            line: ident_line,
            visibility: vis.map_or(Visibility::Private, convert_visibility),
            doc,
            doc_first_line,
            trait_impl: origin.trait_impl,
            lint_allow,
        });
    }

    /// Collects `allow(...)` ids from plain `//` comments that sit between the first
    /// attribute (or the `pub`/`fn` keyword) and the function name, or in the unbroken run
    /// of comment lines directly above the item. Lines are 1-based.
    fn lint_allows(&self, start_line: usize, ident_line: usize) -> Vec<String> {
        let mut allowed = Vec::new();
        let start = start_line.max(1);

        for n in start..ident_line {
            if let Some(line) = self.lines.get(n - 1) {
                if is_plain_comment(line) {
                    parse_allow_marker(line, &mut allowed);
                }
            }
        }

        let mut n = start;
        while n > 1 {
            n -= 1;
            match self.lines.get(n - 1) {
                Some(line) if is_plain_comment(line) => parse_allow_marker(line, &mut allowed),
                _ => break,
            }
        }
        allowed
    }
}

/// First source line of an item: the earliest of its attributes, visibility and signature.
fn item_start_line(
    sig: &syn::Signature,
    vis: Option<&syn::Visibility>,
    attrs: &[Attribute],
) -> usize {
    let mut start = sig.span().start().line;
    for attr in attrs {
        start = start.min(attr.span().start().line);
    }
    if let Some(vis) = vis {
        if !matches!(vis, syn::Visibility::Inherited) {
            start = start.min(vis.span().start().line);
        }
    }
    start
}

fn convert_visibility(vis: &syn::Visibility) -> Visibility {
    match vis {
        syn::Visibility::Public(_) => Visibility::Pub,
        syn::Visibility::Inherited => Visibility::Private,
        syn::Visibility::Restricted(restricted) => {
            let path = restricted
                .path
                .to_token_stream()
                .to_string()
                .replace(' ', "");
            match (restricted.in_token.is_some(), path.as_str()) {
                (false, "crate") => Visibility::PubCrate,
                (false, "super") => Visibility::PubSuper,
                // `pub(self)` is the same as no keyword at all.
                (false, "self") => Visibility::Private,
                _ => Visibility::PubIn(path),
            }
        }
    }
}

/// Joins `#[doc = "..."]` attributes (what `///` and `/** */` become) into one text.
/// Returns `None` when the item has no doc attribute with a string value.
fn doc_text(attrs: &[Attribute]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let Meta::NameValue(name_value) = &attr.meta else {
            continue;
        };
        let Expr::Lit(ExprLit {
            lit: Lit::Str(text),
            ..
        }) = &name_value.value
        else {
            continue;
        };
        found = true;
        for line in text.value().split('\n') {
            let line = line.strip_prefix(' ').unwrap_or(line);
            lines.push(line.trim_end().to_string());
        }
    }

    found.then(|| lines.join("\n"))
}

/// A `//` comment that is neither a doc comment (`///`, `//!`) nor part of code.
fn is_plain_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
}

/// Parses `// codesearch-lint: allow(R001, R002)` and appends the upper-cased ids.
fn parse_allow_marker(line: &str, out: &mut Vec<String>) {
    let Some(after_marker) = line.split_once(ALLOW_MARKER).map(|(_, rest)| rest) else {
        return;
    };
    let Some(after_allow) = after_marker
        .trim_start()
        .strip_prefix("allow(")
        .map(str::to_string)
    else {
        return;
    };
    let Some((ids, _)) = after_allow.split_once(')') else {
        return;
    };
    for id in ids.split(',') {
        let id = id.trim().to_ascii_uppercase();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        // Always a plain function, even when nested inside a method body.
        self.record(&node.sig, Some(&node.vis), &node.attrs, Origin::free());
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let origin = Origin::in_scope(self.scopes.last());
        self.record(&node.sig, Some(&node.vis), &node.attrs, origin);
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        // Trait items carry no visibility keyword.
        let origin = Origin::in_scope(self.scopes.last());
        self.record(&node.sig, None, &node.attrs, origin);
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let target = convert_type(&node.self_ty).to_string();
        let (label, trait_impl) = match &node.trait_ {
            Some((_, path, _)) => {
                let trait_path = convert_path(path).to_string();
                (format!("{trait_path} for {target}"), Some(trait_path))
            }
            None => (target, None),
        };
        self.scopes.push(Scope { label, trait_impl });
        visit::visit_item_impl(self, node);
        self.scopes.pop();
    }

    fn visit_item_trait(&mut self, node: &'ast ItemTrait) {
        self.scopes.push(Scope {
            label: format!("trait {}", node.ident),
            trait_impl: None,
        });
        visit::visit_item_trait(self, node);
        self.scopes.pop();
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        self.inline_mods.push(node.ident.to_string());
        visit::visit_item_mod(self, node);
        self.inline_mods.pop();
    }
}

/// Converts a `syn` type into the shared AST. Lifetimes are dropped.
pub fn convert_type(ty: &Type) -> TypeExpr {
    match ty {
        Type::Paren(p) => convert_type(&p.elem),
        Type::Group(g) => convert_type(&g.elem),
        Type::Tuple(t) if t.elems.is_empty() => TypeExpr::Unit,
        Type::Tuple(t) => TypeExpr::Tuple(t.elems.iter().map(convert_type).collect()),
        Type::Reference(r) => TypeExpr::Ref {
            mutable: r.mutability.is_some(),
            inner: Box::new(convert_type(&r.elem)),
        },
        Type::Slice(s) => TypeExpr::Slice(Box::new(convert_type(&s.elem))),
        Type::Array(a) => TypeExpr::Array(
            Box::new(convert_type(&a.elem)),
            Some(a.len.to_token_stream().to_string()),
        ),
        Type::Path(p) if p.qself.is_none() => convert_path(&p.path),
        Type::ImplTrait(i) => TypeExpr::TraitObject {
            dyn_: false,
            bounds: convert_bounds(&i.bounds),
        },
        Type::TraitObject(t) => TypeExpr::TraitObject {
            dyn_: true,
            bounds: convert_bounds(&t.bounds),
        },
        other => TypeExpr::Other(other.to_token_stream().to_string()),
    }
}

fn convert_bounds(bounds: &Punctuated<TypeParamBound, Token![+]>) -> Vec<TypeExpr> {
    bounds
        .iter()
        .filter_map(|bound| match bound {
            TypeParamBound::Trait(t) => Some(convert_path(&t.path)),
            _ => None, // lifetime bounds etc.
        })
        .collect()
}

fn convert_path(path: &syn::Path) -> TypeExpr {
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();

    let args = match path.segments.last().map(|segment| &segment.arguments) {
        Some(PathArguments::AngleBracketed(angle)) => angle
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(t) => Some(convert_type(t)),
                GenericArgument::Const(expr) => {
                    Some(TypeExpr::Other(expr.to_token_stream().to_string()))
                }
                GenericArgument::AssocType(assoc) => Some(TypeExpr::Other(format!(
                    "{} = {}",
                    assoc.ident,
                    convert_type(&assoc.ty)
                ))),
                _ => None, // lifetimes, constraints
            })
            .collect(),
        // `Fn(A, B) -> C` is modelled as `Fn<(A, B), C>`.
        Some(PathArguments::Parenthesized(paren)) => {
            let inputs = paren.inputs.iter().map(convert_type).collect();
            let output = match &paren.output {
                ReturnType::Default => TypeExpr::Unit,
                ReturnType::Type(_, t) => convert_type(t),
            };
            vec![TypeExpr::Tuple(inputs), output]
        }
        _ => Vec::new(),
    };

    TypeExpr::Path { segments, args }
}
