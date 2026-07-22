//! Compile-time constants and default configuration values.

/// Defines stdout file
pub const STDOUT_DEV: &str = "/dev/stdout";

/// Maximum directory depth to watch
pub const MAX_DIR_DEPTH: usize = 8;

/// Maximum watched files
pub const MAX_OPEN_FILES: usize = 2048;

/// Read tail of this length from large files
pub const TAIL_BYTES: u64 = 1024;

/// Filename glob patterns ignored by default. These are transient files that
/// tools create and immediately rename away (e.g. `rustfmt` writes
/// `foo.rs.tmp.PID.HASH` then renames it over `foo.rs`), plus common editor
/// swap/backup files. Watching them produces confusing diffs under temporary
/// names, so we skip them and let the real file's rename event show the diff.
pub const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    "*.tmp",   // generic temp files
    "*.tmp.*", // rustfmt-style temp files: foo.rs.tmp.29966.0e8daadcf5e2
    "*~",      // emacs / general backup files
    ".*.sw?",  // vim swap files: .foo.swp / .foo.swo / .foo.swx
    "*.orig",  // merge / patch leftovers
    "*.bak",   // backups
];
