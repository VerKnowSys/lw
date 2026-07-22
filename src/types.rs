//! Shared type aliases used across the crate.

use std::collections::HashMap;

/// Per-file watch state: the file's inode number and the last read byte
/// position. The inode lets us detect when a path was replaced by a brand new
/// file (atomic rename, log rotation) so we can re-read it from the start.
pub type FileState = (u64, u64);

/// Maps a watched file path to its [`FileState`].
pub type FileAndPosition = HashMap<String, FileState>;
