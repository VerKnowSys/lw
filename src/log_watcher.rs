//! "lw" log-watcher utility

//! LW docs

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
    const_err,
    dead_code,
    improper_ctypes,
    non_shorthand_field_patterns,
    no_mangle_generic_items,
    overflowing_literals,
    path_statements,
    patterns_in_fns_without_body,
    private_in_public,
    unconditional_recursion,
    unused,
    unused_allocation,
    unused_comparisons,
    unused_parens,
    while_true,
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications
)]

/// Use MiMalloc as default allocator:
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


#[macro_use]
extern crate log;

use kqueue2::{Ident::*, *};
use std::{
    collections::HashMap,
    env,
    fmt::Display,
    fs::{metadata, File},
    io::{prelude::*, BufReader, SeekFrom},
    path::Path,
    process::exit,
};

use chrono::Local;
use colored::Colorize;
use fern::Dispatch;
use lazy_static::lazy_static;
use log::LevelFilter;
use std::sync::Mutex;
use walkdir::WalkDir;

lazy_static! {
    /// List of (to_notify, message, notifier name, webhook) tuples:
    static ref LAST_FILE: Mutex<String> = Mutex::new({
        #[allow(unused_mut)]
        let mut string = String::new();
        string
    });
}


/// FileAndPosition alias type for HashMap of File path and file cursor position (in bytes)
type FileAndPosition = HashMap<String, u64>;

/// Defines stdout file
const STDOUT_DEV: &str = "/dev/stdout";

/// Maximum directory depth to watch
const MAX_DIR_DEPTH: usize = 5;

/// Maximum watched files
const MAX_OPEN_FILES: usize = 1023;


/// Utility to wrap fatal errors
fn fatal<S: Display>(fmt: S) -> ! {
    error!("FATAL ERROR: {}", fmt.to_string().red());
    exit(1)
}


/// Resursively filter out all unreadable/unaccessible/inproper and handle proper files
fn walkdir_recursive(mut kqueue_watcher: &mut Watcher, file_path: &Path) {
    WalkDir::new(&file_path)
        .follow_links(true)
        .min_depth(MIN_DIR_DEPTH)
        .max_depth(MAX_DIR_DEPTH)
        .into_iter()
        .filter_map(|element| element.ok())
        .for_each(|element| watch_file(&mut kqueue_watcher, element.path()));
}


fn main() {
    // Read value of DEBUG from env, if defined switch log level to Debug:
    let loglevel = match env::var("DEBUG") {
        Ok(_) => LevelFilter::Debug,
        Err(_) => LevelFilter::Info,
    };

    // Dispatch logger:
    Dispatch::new()
        .format(move |out, message, _record| {
            out.finish(format_args!(
                "{}: {}",
                Local::now().to_rfc3339().black(),
                message
            ))
        })
        .level(loglevel)
        .chain(File::open(STDOUT_DEV).unwrap_or_else(|_| {
            fatal(format!(
                "{}: STDOUT device {} is not available! Something is terribly wrong here!",
                "FATAL ERROR".red(),
                STDOUT_DEV.yellow()
            ))
        }))
        .apply()
        .unwrap_or_else(|err| {
            fatal(format!(
                "{}: Couldn't initialize Log-Watcher. Details: {}",
                "FATAL ERROR".red(),
                err.to_string().yellow()
            ));
        });

    // mutable hashmap keeping position of all watched files:
    let mut watched_file_states = FileAndPosition::new();

    // mutable kqueue watcher:
    let mut kqueue_watcher = Watcher::new()
        .unwrap_or_else(|e| fatal(format!("Could not create kq watcher: {}", e)));

    // read paths given as arguments:
    let paths_to_watch: Vec<String> = env::args()
        .skip(1) // first arg is $0
        .collect();

    debug!("Watching paths: {}", paths_to_watch.join(", "));
    if paths_to_watch.is_empty() {
        fatal(
            "No paths specified as arguments! You have to specify at least a single directory/file to watch!",
        );
    }

    // initial watches for specified dirs/files:
    {
        paths_to_watch.into_iter().for_each(|a_path| {
            // Handle case when given a file as argument
            let file_path = Path::new(&a_path);
            watch_file(&mut kqueue_watcher, &file_path);
            walkdir_recursive(&mut kqueue_watcher, &file_path);
        });
    }

    if kqueue_watcher.watch().is_ok() {
        // handle events dynamically, including new files
        while let Some(an_event) = kqueue_watcher.iter().next() {
            match an_event.ident {
                Filename(_file_descriptor, abs_file_name) => {
                    let file_path = Path::new(&abs_file_name);
                    match metadata(file_path) {
                        Ok(metadata) => {
                            if metadata.is_dir() {
                                // handle dirs
                                debug!("{}: {}", "+DirLoad".magenta(), abs_file_name.cyan());
                                walkdir_recursive(&mut kqueue_watcher, file_path);
                                kqueue_watcher.watch().unwrap_or_default();
                            } else {
                                // handle files
                                debug!("{}: {}", "+New".magenta(), abs_file_name.cyan());
                                watch_file(&mut kqueue_watcher, file_path);
                                kqueue_watcher.watch().unwrap_or_default();
                                handle_file_event(&mut watched_file_states, &abs_file_name);
                            }
                        }

                        Err(error_cause) => {
                            // handle situation when logs are wiped out and unavailable to read anymore
                            debug!("{}: {}", "-Watch".magenta(), abs_file_name.cyan());
                            kqueue_watcher
                                .remove_filename(file_path, EventFilter::EVFILT_VNODE)
                                .unwrap_or_else(|error| {
                                    error!(
                                        "Could not remove watch on file: {:?}. Error cause: {}",
                                        abs_file_name.cyan(),
                                        error.to_string().red()
                                    )
                                });
                            // try to build list if path exists
                            if file_path.exists() {
                                walkdir_recursive(&mut kqueue_watcher, file_path);
                                kqueue_watcher.watch().unwrap_or_default();
                            } else {
                                error!(
                                    "Dropped watch on file/dir: {}. Error cause: {}",
                                    format!("{:?}", &file_path).red(),
                                    format!("{}", &error_cause).red()
                                );
                            }
                        }
                    };
                }

                event => warn!("Unknown event: {:?}", event),
            }
        }
    }
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
fn watch_file(kqueue_watcher: &mut Watcher, file: &Path) {
    kqueue_watcher
        .remove_filename(file, EventFilter::EVFILT_VNODE)
        .map(|e| {
            debug!("{}: {:?}", "-Watch".magenta(), file);
            e
        })
        .unwrap_or_default();
    kqueue_watcher
        .add_filename(
            &file,
            EventFilter::EVFILT_VNODE,
            NOTE_WRITE
                | NOTE_LINK
                | NOTE_RENAME
                | NOTE_DELETE
                | NOTE_EXTEND
                | NOTE_ATTRIB
                | NOTE_REVOKE,
        )
        .map(|e| {
            debug!("{}: {}", "+Watch".magenta(), format!("{:?}", file).cyan());
            e
        })
        .unwrap_or_else(|error_cause| {
            error!(
                "Could not watch file {:?}. Error cause: {}",
                file,
                error_cause.to_string().red()
            )
        });
}


/// Handle action triggered by an event
fn handle_file_event(states: &mut FileAndPosition, file_path: &str) {
    let watched_file = file_path.to_string();
    let file_position = states.entry(watched_file.clone()).or_insert(0);
    {
        debug!(
            "{}: {} {}",
            "+EventHandle".magenta(),
            watched_file.cyan(),
            format!("@{}", file_position).black()
        );
        let file_size = match metadata(&watched_file) {
            Ok(file_metadata) => file_metadata.len(),
            Err(_) => 0,
        };

        // print header only when file is at beginning and not often than N bytes after previous one (limits header spam)
        if *file_position == 0 || *LAST_FILE.lock().unwrap() != watched_file {
            println!();
            println!(); // just start new entry after \n\n
            info!("{}", watched_file.blue());
        }

        // print content of file that triggered the event
        if *file_position < file_size {
            let content = seek_file_to_position_and_read(&watched_file, *file_position);
            println!("{}", content.join("\n"));
            states.insert(watched_file.clone(), file_size);
        }
    }

    *LAST_FILE.lock().unwrap() = watched_file;
}


/// Set file position in bytes and print new file contents
fn seek_file_to_position_and_read(file_to_watch: &str, file_position: u64) -> Vec<String> {
    match File::open(&file_to_watch) {
        Ok(some_file) => {
            let mut cursor = BufReader::new(some_file);
            cursor
                .seek(SeekFrom::Start(file_position))
                .unwrap_or_else(|_| 0);
            cursor.lines().filter_map(|line| line.ok()).collect()
        }

        Err(error_cause) => {
            error!(
                "Couldn't open file: {}. Error cause: {}",
                file_to_watch.yellow(),
                error_cause.to_string().red()
            );
            vec![]
        }
    }
}
