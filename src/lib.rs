//! "lw" log-watcher library.
//!
//! Shared building blocks used by the `lw` binary: configuration loading,
//! constants, shared types and the watcher/event-handling utilities.

#![forbid(unsafe_code)]
#![deny(
    missing_docs,
    unstable_features,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications,
    bad_style,
    dead_code,
    improper_ctypes,
    non_shorthand_field_patterns,
    no_mangle_generic_items,
    overflowing_literals,
    path_statements,
    patterns_in_fns_without_body,
    unconditional_recursion,
    unused,
    unused_allocation,
    unused_comparisons,
    unused_parens,
    while_true,
    unused_extern_crates
)]

#[macro_use]
extern crate log;

pub mod config;
pub mod consts;
pub mod types;
pub mod utils;
