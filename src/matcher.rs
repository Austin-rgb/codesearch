//! Structural matching of a [`Query`] against an indexed [`FunctionSig`].
//!
//! Matching is purely syntactic. The one extension point is [`TypeResolver`]:
//! the matcher passes every candidate type through it before comparing, so
//! type-alias resolution, `Self` substitution, or trait-aware normalisation can be
//! added later by providing a different resolver, without touching the query
//! parser or [`type_matches`].

use crate::types::{FunctionSig, Param, Query, TypeExpr};
use std::borrow::Cow;

/// Rewrites a candidate type before it is compared with the query.
///
/// Implementations are responsible for recursing into nested types if they need to.
/// A resolver that changes the *head* of a type (e.g. expanding an alias) should
/// not be combined with the SQL pre-filter in `Index::for_each_candidate`.
pub trait TypeResolver {
    fn resolve<'a>(&self, ty: &'a TypeExpr, _context: &FunctionSig) -> Cow<'a, TypeExpr> {
        Cow::Borrowed(ty)
    }
}

/// The prototype's resolver: leaves every type exactly as written.
#[derive(Debug, Default, Clone, Copy)]
pub struct SyntacticResolver;

impl TypeResolver for SyntacticResolver {}

#[derive(Debug, Default, Clone, Copy)]
pub struct MatchOptions {
    /// Ignore a `self` receiver when comparing parameters, so
    /// `UserId -> User` finds `fn get(&self, id: UserId) -> User`.
    pub ignore_self: bool,
}

pub struct Matcher {
    resolver: Box<dyn TypeResolver>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new(Box::new(SyntacticResolver))
    }
}

impl Matcher {
    pub fn new(resolver: Box<dyn TypeResolver>) -> Self {
        Self { resolver }
    }

    /// True when `func` has the same parameter count, parameter types (in order)
    /// and return type as `query`, with `*` matching any single type.
    pub fn matches(&self, query: &Query, func: &FunctionSig, opts: MatchOptions) -> bool {
        let params: &[Param] = if opts.ignore_self && func.has_self {
            &func.params[1..]
        } else {
            &func.params
        };

        if params.len() != query.params.len() {
            return false;
        }

        for (wanted, param) in query.params.iter().zip(params) {
            let actual = self.resolver.resolve(&param.ty, func);
            if !type_matches(wanted, &actual) {
                return false;
            }
        }

        let ret = self.resolver.resolve(&func.ret, func);
        type_matches(&query.ret, &ret)
    }
}

/// Recursive structural comparison of a query type against a candidate type.
///
/// * `*` matches any single type expression.
/// * Named types match when one path is a suffix of the other (`Session` ~ `auth::Session`)
///   and their generic arguments match pairwise with equal counts.
/// * References match only references with the same mutability.
pub fn type_matches(query: &TypeExpr, candidate: &TypeExpr) -> bool {
    match (query, candidate) {
        (TypeExpr::Wildcard, _) => true,
        (TypeExpr::Unit, TypeExpr::Unit) => true,
        (TypeExpr::Tuple(q), TypeExpr::Tuple(c)) => all_match(q, c),
        (
            TypeExpr::Path {
                segments: qs,
                args: qa,
            },
            TypeExpr::Path {
                segments: cs,
                args: ca,
            },
        ) => segments_match(qs, cs) && all_match(qa, ca),
        (
            TypeExpr::Ref {
                mutable: qm,
                inner: qi,
            },
            TypeExpr::Ref {
                mutable: cm,
                inner: ci,
            },
        ) => qm == cm && type_matches(qi, ci),
        (TypeExpr::Slice(q), TypeExpr::Slice(c)) => type_matches(q, c),
        (TypeExpr::Array(qi, ql), TypeExpr::Array(ci, cl)) => {
            type_matches(qi, ci) && (ql.is_none() || ql == cl)
        }
        (
            TypeExpr::TraitObject {
                dyn_: qd,
                bounds: qb,
            },
            TypeExpr::TraitObject {
                dyn_: cd,
                bounds: cb,
            },
        ) => qd == cd && all_match(qb, cb),
        (TypeExpr::Other(q), TypeExpr::Other(c)) => q == c,
        _ => false,
    }
}

fn all_match(query: &[TypeExpr], candidate: &[TypeExpr]) -> bool {
    query.len() == candidate.len() && query.iter().zip(candidate).all(|(q, c)| type_matches(q, c))
}

/// `Session` matches `auth::Session`, but `a::Session` does not match `b::Session`.
fn segments_match(query: &[String], candidate: &[String]) -> bool {
    let (short, long) = if query.len() <= candidate.len() {
        (query, candidate)
    } else {
        (candidate, query)
    };
    long.ends_with(short)
}
