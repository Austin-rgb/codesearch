//! `codesearch`: find Rust functions by their parameter and return types.
//!
//! The crate is split so each stage can be replaced independently:
//!
//! ```text
//! query string ──parser──▶ Query ─┐
//!                                 ├─matcher─▶ matching FunctionSigs ──search──▶ output
//! .rs files ──extractor──▶ FunctionSig ──index (SQLite)─┘
//! ```
//!
//! `types` holds the shared type-expression AST that every stage speaks.

pub mod cli;
pub mod extractor;
pub mod index;
pub mod lint;
pub mod matcher;
pub mod parser;
pub mod search;
pub mod types;
