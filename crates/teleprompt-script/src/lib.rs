//! The script language: a script parsed into its syntax tree ([`ast`],
//! [`parse`]), its ids checked ([`ident`]), resolved against the project's
//! config into a [`program::Program`], linted, rewritten in place for an
//! edit ([`edit`]), and translated through a sidecar ([`translation`]).
//!
//! The words every crate shares, ids, times, attributes and the config, are
//! `teleprompt-core`'s; this crate is only for what reads a script.

pub mod ast;
pub mod edit;
pub mod ident;
pub mod lint;
pub mod parse;
pub mod program;
pub mod translation;
