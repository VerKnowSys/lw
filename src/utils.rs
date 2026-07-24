//! Utility functions: directory walking, kqueue watch management, event
//! handling and the pure decision helpers that drive them.

use crate::config::Config;
use crate::types::{FileAndPosition, FileState};
use colored::Colorize;
use kqueue2::*;
use std::{
    fs::{File, OpenOptions, metadata},
    io::{BufReader, IsTerminal, SeekFrom, Write, prelude::*},
    os::unix::fs::MetadataExt,
    path::Path,
};
use walkdir::WalkDir;


/// Write-once-and-atomic to a file
pub fn write_append(file_path: &str, contents: &str) {
    if !contents.is_empty() {
        match OpenOptions::new().create(true).append(true).open(file_path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes()).unwrap_or_else(|_| {
                    panic!("Access denied? File can't be written: {file_path}")
                });
                debug!("Atomically written data to file: {file_path}");
            }

            Err(err) => {
                error!("Atomic write to: {file_path} has failed! Cause: {err}")
            }
        }
    }
}


/// Minimal shell-style glob match supporting `*` (any run of characters) and
/// `?` (a single character). Matches the whole `name` against `pattern`.
///
/// Pure recursive descent over the two char slices:
/// - `*` matches zero characters (advance the pattern), or one character then
///   retries `*` (advance the name);
/// - `?` matches exactly one character;
/// - any other character matches itself.
fn glob_match(name: &[char], pattern: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((&'*', rest)) => {
            glob_match(name, rest)
                || matches!(name.split_first(), Some((_, tail)) if glob_match(tail, pattern))
        }
        Some((&'?', rest)) => {
            matches!(name.split_first(), Some((_, tail)) if glob_match(tail, rest))
        }
        Some((&expected, rest)) => {
            matches!(name.split_first(), Some((&first, tail)) if first == expected && glob_match(tail, rest))
        }
    }
}


/// Convenience wrapper over [`glob_match`] for whole `&str` inputs (test-only).
#[cfg(test)]
fn matches_glob(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    glob_match(&name, &pattern)
}


/// Whether the file at `path` should be ignored, i.e. its file name matches any
/// of the given glob `patterns`. Used to skip transient temp/swap/backup files.
fn is_ignored(path: &Path, patterns: &[String]) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        // Collect the file name's chars once, then test each pattern against it.
        Some(name) => {
            let name: Vec<char> = name.chars().collect();
            patterns
                .iter()
                .any(|pattern| glob_match(&name, &pattern.chars().collect::<Vec<_>>()))
        }
        None => false,
    }
}


/// Resursively filter out all unreadable/unaccessible/inproper and handle proper files
pub fn walkdir_recursive(
    kqueue_watcher: &mut Watcher,
    watched_file_states: &mut FileAndPosition,
    last_file: &mut String,
    file_path: &Path,
    config: &Config,
) {
    let ignore_patterns = config.ignore_patterns.as_deref().unwrap_or_default();
    WalkDir::new(file_path)
        .same_file_system(false)
        .contents_first(true)
        .follow_links(config.follow_links.unwrap_or_default())
        .max_open(config.max_open_files.unwrap_or_default())
        .max_depth(config.max_dir_depth.unwrap_or_default())
        .into_iter()
        .filter_map(|element| element.ok())
        .filter(|element| !is_ignored(element.path(), ignore_patterns))
        .for_each(|element| {
            watch_file(
                kqueue_watcher,
                watched_file_states,
                last_file,
                element.path(),
            )
        });
}


/// Process file with event
pub fn process_file_event(
    abs_file_name: &str,
    kqueue_watcher: &mut Watcher,
    watched_file_states: &mut FileAndPosition,
    last_file: &mut String,
    config: &Config,
) {
    let file_path = Path::new(&abs_file_name);
    // Skip transient temp/swap/backup files (e.g. rustfmt's `foo.rs.tmp.PID`);
    // the real file's own rename event shows the diff under its proper name.
    if is_ignored(
        file_path,
        config.ignore_patterns.as_deref().unwrap_or_default(),
    ) {
        trace!("{}: {}", "-Ignored".magenta(), abs_file_name.cyan());
        return;
    }
    match metadata(file_path) {
        Ok(file_metadata) => {
            if file_metadata.is_dir() {
                trace!("{}: {}", "+DirLoad".magenta(), abs_file_name.cyan());
                walkdir_recursive(
                    kqueue_watcher,
                    watched_file_states,
                    last_file,
                    file_path,
                    config,
                );
            } else {
                trace!("{}: {}", "+FileWatchHandle".magenta(), abs_file_name.cyan());
                calculate_position_and_handle(
                    file_metadata.ino(),
                    file_metadata.len(),
                    watched_file_states,
                    abs_file_name,
                    last_file,
                    config,
                );
            }
        }

        Err(error_cause) => {
            // handle situation when logs are wiped out and unavailable to read anymore
            kqueue_watcher
                .remove_filename(file_path, EventFilter::EVFILT_VNODE)
                .inspect(|_| {
                    trace!("{}: {}", "-Watch".magenta(), abs_file_name.cyan());
                })
                .unwrap_or_else(|error| {
                    error!(
                        "Could not remove watch on file: {:?}. Error cause: {}",
                        abs_file_name.cyan(),
                        error.to_string().red()
                    )
                });
            // try to build list if path exists
            if file_path.exists() {
                if file_path.is_dir() {
                    trace!("{}: {}", "+DirLoad".magenta(), abs_file_name.cyan());
                    walkdir_recursive(
                        kqueue_watcher,
                        watched_file_states,
                        last_file,
                        file_path,
                        config,
                    );
                } else if file_path.is_file() {
                    watch_file(kqueue_watcher, watched_file_states, last_file, file_path);
                }
            } else {
                debug!(
                    "Dropped watch on file/dir: {}. Last value: {}. Error cause: {}",
                    format!("{file_path:?}").cyan(),
                    format!(
                        "{:?}",
                        watched_file_states
                            .remove(abs_file_name)
                            .unwrap_or_default()
                    )
                    .cyan(),
                    format!("{error_cause}").red()
                );
            }
        }
    };
    debug!(
        "Watched files list: [{}]",
        format!("{watched_file_states:?}").cyan()
    );
}


/// Decide which byte offset to start reading a file from, given what we knew
/// about it before this event (`previous`), its current `inode` and `file_size`,
/// and the configured `tail_bytes`.
///
/// - Known file, same inode, cursor within bounds -> continue (show only the
///   newly appended data).
/// - Known file, same inode, cursor past EOF -> the file was truncated, re-read
///   it from the start.
/// - Known path but a different inode -> the file was replaced (atomic rename /
///   log rotation), re-read it from the start.
/// - Never seen before -> skip to the tail so we don't dump the whole
///   pre-existing content (mirrors `tail -F` behaviour).
fn decide_read_position(
    previous: Option<FileState>,
    inode: u64,
    file_size: u64,
    tail_bytes: u64,
) -> u64 {
    match previous {
        Some((last_inode, last_position)) if last_inode == inode => {
            if last_position > file_size {
                0
            } else {
                last_position
            }
        }
        Some(_) => 0,
        None => file_size.saturating_sub(tail_bytes),
    }
}


/// Process file position and handle the event
fn calculate_position_and_handle(
    inode: u64,
    file_size: u64,
    watched_file_states: &mut FileAndPosition,
    abs_file_name: &str,
    last_file: &mut String,
    config: &Config,
) {
    let position = decide_read_position(
        watched_file_states.get(abs_file_name).copied(),
        inode,
        file_size,
        config.tail_bytes.unwrap_or_default(),
    );

    handle_file_event(position, file_size, abs_file_name, last_file);

    // Record the current inode and end offset so the next event shows only
    // newly added data (or a full re-read if the file is replaced/truncated).
    watched_file_states.insert(abs_file_name.to_string(), (inode, file_size));
}


/// Kqueue wrapper for watch()
pub fn watch_the_watcher(kqueue_watcher: &mut Watcher) {
    trace!("{}: watch()", "+Trigger".magenta());
    kqueue_watcher.watch().unwrap_or_default();
}


/// kqueue flags, from: /usr/include/sys/event.h
/// NOTE_DELETE     0x00000001              /* vnode was removed */
/// NOTE_WRITE      0x00000002              /* data contents changed */
/// NOTE_EXTEND     0x00000004              /* size increased */
/// NOTE_ATTRIB     0x00000008              /* attributes changed */
/// NOTE_LINK       0x00000010              /* link count changed */
/// NOTE_RENAME     0x00000020              /* vnode was renamed */
/// NOTE_REVOKE     0x00000040              /* vnode access was revoked */
///
/// Add watch on specified file path
fn watch_file(
    kqueue_watcher: &mut Watcher,
    watched_file_states: &mut FileAndPosition,
    last_file: &mut String,
    file: &Path,
) {
    // Seed/refresh per-file state:
    // - brand new path -> only seed the current size, so startup (and directory
    //   re-walks) don't dump the whole content of every existing file;
    // - known path whose inode changed -> the file was replaced by an atomic
    //   rename / rotation (e.g. `rustfmt` renaming its temp file over the
    //   original), so show its new content from the start under the real name.
    //   This makes replacement detection work even when the file's own vnode
    //   event is lost to the concurrent directory re-walk.
    if let Ok(file_metadata) = metadata(file)
        && file_metadata.is_file()
    {
        let inode = file_metadata.ino();
        let size = file_metadata.len();
        let key = file.to_string_lossy().to_string();
        match watched_file_states.get(&key).copied() {
            Some((stored_inode, _)) if stored_inode != inode => {
                handle_file_event(0, size, &key, last_file);
                watched_file_states.insert(key, (inode, size));
            }
            Some(_) => {}
            None => {
                watched_file_states.insert(key, (inode, size));
            }
        }
    }
    kqueue_watcher
        .remove_filename(file, EventFilter::EVFILT_VNODE)
        .inspect(|_| {
            trace!("{}: {}", "-Watch".magenta(), format!("{file:?}").cyan());
        })
        .unwrap_or_default();
    kqueue_watcher
        .add_filename(
            file,
            EventFilter::EVFILT_VNODE,
            NOTE_WRITE | NOTE_LINK | NOTE_RENAME | NOTE_DELETE | NOTE_EXTEND, // | NOTE_ATTRIB
                                                                              // | NOTE_REVOKE,
        )
        .inspect(|_| {
            trace!("{}: {}", "+Watch".magenta(), format!("{file:?}").cyan());
        })
        .unwrap_or_else(|error_cause| {
            error!(
                "Could not watch file: {}. Caused by: {}",
                format!("{file:?}").cyan(),
                error_cause.to_string().red()
            )
        });
}


/// Whether to print the file header line for this event. We show it when the
/// file is read from its start, or when the previously printed file differs
/// (this limits header spam for consecutive appends to the same file).
fn should_print_header(file_position: u64, last_file: &str, watched_file: &str) -> bool {
    file_position == 0 || last_file != watched_file
}


/// Handle action triggered by an event
fn handle_file_event(
    file_position: u64,
    file_size: u64,
    file_path: &str,
    last_file: &mut String,
) {
    let watched_file = file_path.to_string();

    debug!(
        "Watched file position: {}, file size: {}, file name: {}",
        format!("{file_position}").cyan(),
        format!("{file_size}").cyan(),
        watched_file.cyan()
    );
    trace!(
        "{}: {} {}",
        "+EventHandle".magenta(),
        watched_file.cyan(),
        format!("@{file_position}").black()
    );

    if should_print_header(file_position, last_file, &watched_file) {
        println!();
        println!(); // just start new entry after \n\n
        info!(
            "{} {}",
            watched_file.blue(),
            format!("@{file_position}").black()
        );
    }

    // print content of the file that triggered the event
    if file_position < file_size {
        let content = seek_file_to_position_and_read(&watched_file, file_position);
        println!("{}", render_content(&watched_file, content).join("\n"));
    }

    *last_file = watched_file;
}


/// Syntax-highlight file content for terminal output, keyed on the file
/// extension. When stdout is not a terminal (piped / redirected) the raw lines
/// are returned unchanged, so captured output stays free of ANSI escapes.
fn render_content(file_path: &str, lines: Vec<String>) -> Vec<String> {
    if !std::io::stdout().is_terminal() {
        return lines;
    }
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    crate::highlight::highlighter().highlight(extension, &lines)
}


/// Set file position in bytes and print new file contents
fn seek_file_to_position_and_read(file_to_watch: &str, file_position: u64) -> Vec<String> {
    match File::open(file_to_watch) {
        Ok(some_file) => {
            let mut cursor = BufReader::new(some_file);
            cursor.seek(SeekFrom::Start(file_position)).unwrap_or(0);
            let lines_out: Vec<_> = cursor.lines().map_while(Result::ok).collect();
            trace!("Lines out: '{}'", format!("{lines_out:?}").cyan());
            if lines_out.is_empty() {
                vec![String::from("* binary file modification *")]
            } else {
                lines_out
            }
        }

        Err(error_cause) => {
            error!(
                "Couldn't open file: {}. Caused by: {}",
                file_to_watch.cyan(),
                error_cause.to_string().red()
            );
            vec![]
        }
    }
}


#[cfg(test)]
mod tests {
    use super::{
        decide_read_position, is_ignored, matches_glob, seek_file_to_position_and_read,
        should_print_header,
    };
    use crate::consts::DEFAULT_IGNORE_PATTERNS;
    use std::fs;
    use std::path::Path;

    /// The built-in ignore patterns as owned strings (as a live `Config` holds them).
    fn default_patterns() -> Vec<String> {
        DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|pattern| pattern.to_string())
            .collect()
    }

    /// Bytes to keep from the tail of a never-before-seen file (matches the
    /// default config value used across these tests).
    const TAIL_BYTES: u64 = 2048;

    /// Build a unique temporary file path so tests don't collide with each
    /// other or across parallel runs.
    fn temp_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("lw_test_{}_{}", std::process::id(), name));
        path.to_string_lossy().to_string()
    }

    // ---- decide_read_position: one test per reproduction scenario ----

    #[test]
    fn first_sight_large_file_skips_to_tail() {
        // Never seen before and bigger than tail_bytes: start `tail_bytes`
        // before EOF so we don't dump the whole pre-existing file.
        assert_eq!(
            decide_read_position(None, 1, 5000, TAIL_BYTES),
            5000 - TAIL_BYTES
        );
    }

    #[test]
    fn first_sight_small_file_reads_from_start() {
        // Smaller than tail_bytes: saturating_sub keeps us at the beginning.
        assert_eq!(decide_read_position(None, 1, 100, TAIL_BYTES), 0);
    }

    #[test]
    fn append_continues_from_last_offset() {
        // Same inode, file grew: resume from the previous end -> only the newly
        // appended bytes are shown.
        assert_eq!(decide_read_position(Some((1, 24)), 1, 35, TAIL_BYTES), 24);
    }

    #[test]
    fn no_growth_returns_end_so_nothing_is_reprinted() {
        // Same inode, size unchanged: position == size, so the caller's
        // `position < size` guard prints nothing (no duplicate output).
        assert_eq!(decide_read_position(Some((1, 35)), 1, 35, TAIL_BYTES), 35);
    }

    #[test]
    fn truncate_in_place_smaller_rereads_from_start() {
        // Same inode but the cursor is now past EOF -> file was truncated.
        assert_eq!(decide_read_position(Some((1, 800)), 1, 23, TAIL_BYTES), 0);
    }

    #[test]
    fn rewrite_in_place_larger_resumes_from_offset() {
        // Same inode, still growing past the old cursor: like `tail -F`, we
        // can't tell an in-place rewrite from an append, so we resume from the
        // old offset. Documented limitation.
        assert_eq!(decide_read_position(Some((1, 24)), 1, 48, TAIL_BYTES), 24);
    }

    #[test]
    fn replaced_file_new_inode_rereads_from_start() {
        // Atomic rename / rotation: same path, different inode -> read the
        // whole new file from the beginning.
        assert_eq!(decide_read_position(Some((1, 800)), 2, 40, TAIL_BYTES), 0);
    }

    #[test]
    fn append_after_replace_continues_from_offset() {
        // Once the new inode is recorded, subsequent appends resume normally.
        assert_eq!(decide_read_position(Some((2, 40)), 2, 63, TAIL_BYTES), 40);
    }

    // ---- should_print_header: when to emit the file header line ----

    #[test]
    fn header_printed_when_reading_from_start() {
        // Reading from offset 0 always shows the header, even for the same file.
        assert!(should_print_header(0, "sub/app.log", "sub/app.log"));
    }

    #[test]
    fn header_printed_when_file_differs_from_last() {
        assert!(should_print_header(42, "sub/other.log", "sub/app.log"));
    }

    #[test]
    fn header_suppressed_for_consecutive_appends_to_same_file() {
        // Same file, not at the start: suppress the header to avoid spam.
        assert!(!should_print_header(42, "sub/app.log", "sub/app.log"));
    }

    // ---- matches_glob / is_ignored: temp-file filtering ----

    #[test]
    fn glob_star_matches_any_run() {
        assert!(matches_glob("foo.rs", "*.rs"));
        assert!(matches_glob("foo.rs", "*"));
        assert!(matches_glob("foo.tmp", "*.tmp"));
        assert!(!matches_glob("foo.rs", "*.tmp"));
    }

    #[test]
    fn glob_question_matches_single_char() {
        assert!(matches_glob(".foo.swp", ".*.sw?"));
        assert!(matches_glob(".foo.swo", ".*.sw?"));
        assert!(!matches_glob(".foo.sw", ".*.sw?")); // `?` needs exactly one char
    }

    #[test]
    fn glob_handles_embedded_star() {
        // rustfmt temp file: name.ext.tmp.PID.HASH
        assert!(matches_glob(
            "log_watcher.rs.tmp.29966.0e8daadcf5e2",
            "*.tmp.*"
        ));
        assert!(!matches_glob("log_watcher.rs", "*.tmp.*"));
    }

    #[test]
    fn rustfmt_temp_file_is_ignored_but_real_file_is_not() {
        let patterns = default_patterns();
        assert!(is_ignored(
            Path::new("./src/log_watcher.rs.tmp.29966.0e8daadcf5e2"),
            &patterns
        ));
        assert!(!is_ignored(Path::new("./src/log_watcher.rs"), &patterns));
    }

    #[test]
    fn common_editor_temp_files_are_ignored() {
        let patterns = default_patterns();
        for name in ["notes.txt~", ".main.rs.swp", "patch.orig", "data.bak"] {
            assert!(
                is_ignored(Path::new(name), &patterns),
                "should ignore {name}"
            );
        }
    }

    #[test]
    fn empty_patterns_ignore_nothing() {
        assert!(!is_ignored(Path::new("foo.rs.tmp.1.2"), &[]));
    }

    // ---- seek_file_to_position_and_read: content extraction ----

    #[test]
    fn reads_whole_file_from_start() {
        let path = temp_path("read_all");
        fs::write(&path, "line1\nline2\nline3\n").unwrap();
        assert_eq!(
            seek_file_to_position_and_read(&path, 0),
            vec!["line1", "line2", "line3"]
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reads_only_appended_tail_from_offset() {
        let path = temp_path("read_offset");
        fs::write(&path, "line1\nline2\nline3\n").unwrap();
        // "line1\n" == 6 bytes: reading from offset 6 yields only what follows.
        assert_eq!(
            seek_file_to_position_and_read(&path, 6),
            vec!["line2", "line3"]
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reading_at_eof_yields_binary_marker() {
        let path = temp_path("read_eof");
        fs::write(&path, "only line\n").unwrap();
        // Nothing left to read past EOF -> the sentinel message.
        assert_eq!(
            seek_file_to_position_and_read(&path, 10),
            vec!["* binary file modification *"]
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_returns_empty() {
        let path = temp_path("does_not_exist");
        let _ = fs::remove_file(&path);
        assert!(seek_file_to_position_and_read(&path, 0).is_empty());
    }

    /// End-to-end of the reported bug: a large file gets rewritten to a smaller
    /// size, so the stored cursor is past the new EOF. We must reset to 0 and
    /// read the full new content (previously produced a header with no body).
    #[test]
    fn truncated_rewrite_shows_full_new_content() {
        let path = temp_path("truncate_flow");
        // Pretend we had watched ~800 bytes of old content.
        let previous = Some((1u64, 800u64));
        // File is rewritten in place (same inode) to something small.
        fs::write(&path, "NEW SMALL CONTENT LINE\n").unwrap();
        let new_size = fs::metadata(&path).unwrap().len();
        let position = decide_read_position(previous, 1, new_size, TAIL_BYTES);
        assert_eq!(position, 0, "truncation must reset the cursor to the start");
        assert_eq!(
            seek_file_to_position_and_read(&path, position),
            vec!["NEW SMALL CONTENT LINE"]
        );
        let _ = fs::remove_file(&path);
    }
}
