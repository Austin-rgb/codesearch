//! Extracts function signatures from Rust source with `syn`.
//!
//! Everything is purely syntactic: macros are not expanded, `cfg` attributes are
//! ignored, type aliases are not resolved, and lifetimes / `where` clauses are dropped.
//!
//! To support another language, write a sibling module that produces the same
//! [`FunctionSig`] values; the query parser, matcher and index do not care where
//! signatures come from.

use crate::types::{FnKind, FunctionSig, Param, TypeExpr};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{
    FnArg, GenericArgument, GenericParam, ImplItemFn, ItemFn, ItemImpl, ItemMod, ItemTrait, Pat,
    PathArguments, ReturnType, Token, TraitItemFn, Type, TypeParamBound,
};

/// Parses `source` and returns every function and method it declares.
///
/// `rel_path` is the file's path relative to the project root (using `/`); it is
/// stored on each result and used to derive the module path.
pub fn extract_functions(source: &str, rel_path: &str) -> syn::Result<Vec<FunctionSig>> {
    let file = syn::parse_file(source)?;
    let mut collector = Collector {
        file: rel_path,
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

struct Collector<'a> {
    file: &'a str,
    base_module: String,
    inline_mods: Vec<String>,
    /// Innermost `impl` / `trait` label, if any.
    scopes: Vec<String>,
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

    fn record(&mut self, sig: &syn::Signature, kind: FnKind, impl_target: Option<String>) {
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

        self.out.push(FunctionSig {
            name: sig.ident.to_string(),
            module: self.module(),
            kind,
            impl_target,
            has_self,
            generics,
            params,
            ret,
            file: self.file.to_string(),
            line: sig.ident.span().start().line,
        });
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        // Always a plain function, even when nested inside a method body.
        self.record(&node.sig, FnKind::Function, None);
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let target = self.scopes.last().cloned();
        self.record(&node.sig, FnKind::Method, target);
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        let target = self.scopes.last().cloned();
        self.record(&node.sig, FnKind::Method, target);
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let target = convert_type(&node.self_ty).to_string();
        let label = match &node.trait_ {
            Some((_, path, _)) => format!("{} for {}", convert_path(path), target),
            None => target,
        };
        self.scopes.push(label);
        visit::visit_item_impl(self, node);
        self.scopes.pop();
    }

    fn visit_item_trait(&mut self, node: &'ast ItemTrait) {
        self.scopes.push(format!("trait {}", node.ident));
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
