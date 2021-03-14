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

use config::Config;
use kqueue2::{Ident::*, *};
use std::{
    collections::HashMap,
    env,
    fs::{metadata, File},
    io::{prelude::*, BufReader, SeekFrom},
    path::Path,
    process::exit,
};

use chrono::Local;
use colored::Colorize;
use fern::Dispatch;
use walkdir::WalkDir;


mod config;


/// FileAndPosition alias type for HashMap of File path and file cursor position (in bytes)
type FileAndPosition = HashMap<String, u64>;


/// Resursively filter out all unreadable/unaccessible/inproper and handle proper files
fn walkdir_recursive(mut kqueue_watcher: &mut Watcher, file_path: &Path, config: &Config) {
    WalkDir::new(&file_path)
        .contents_first(true)
        .follow_links(config.follow_links.unwrap_or_default())
        .max_open(config.max_open_files.unwrap_or_default())
        .max_depth(config.max_dir_depth.unwrap_or_default())
        .into_iter()
        .filter_map(|element| element.ok())
        .for_each(|element| watch_file(&mut kqueue_watcher, element.path()));
}


fn main() {
    let config = Config::load();
    let log_level = config.get_log_level();
    let output = config.output.clone().unwrap_or_default();

    // read paths given as arguments:
    let paths_to_watch: Vec<String> = env::args()
        .skip(1) // first arg is $0
        .collect();

    // mutable hashmap keeping position of all watched files:
    let mut watched_file_states = FileAndPosition::new();

    // mutable kqueue watcher:
    let mut kqueue_watcher = Watcher::new().expect("Could not create kq watcher!");

    // name of the last logged file:
    let mut last_file = String::new();

    // Dispatch logger:
    Dispatch::new()
        .format(|out, message, _record| {
            out.finish(format_args!(
                "{}: {}",
                Local::now().to_rfc3339().black(),
                message
            ))
        })
        .level(log_level)
        .chain(File::open(output.clone()).unwrap_or_else(|_| {
            panic!("{}: Couldn't open: {}!", "FATAL ERROR".red(), output.cyan())
        }))
        .apply()
        .expect("Couldn't initialize Fern logger!");

    debug!("Watching paths: {}", paths_to_watch.join(", "));
    if paths_to_watch.is_empty() {
        error!("FATAL ERROR: {}", "No paths specified as arguments! You have to specify at least a single directory/file to watch!".red());
        exit(1)
    }

    // initial watches for specified dirs/files:
    paths_to_watch.into_iter().for_each(|a_path| {
        // Handle case when given a file as argument
        walkdir_recursive(&mut kqueue_watcher, &Path::new(&a_path), &config);
    });

    // handle events dynamically, including new files
    loop {
        watch_the_watcher(&mut kqueue_watcher);
        while let Some(an_event) = kqueue_watcher.iter().next() {
            debug!("Watched files: {}", watched_file_states.iter().count());
            match an_event.ident {
                Filename(_file_descriptor, abs_file_name) => {
                    process_file_event(
                        &abs_file_name,
                        &mut kqueue_watcher,
                        &mut watched_file_states,
                        &mut last_file,
                        &config,
                    );
                    // handle_config_changes(&mut log_level);
                    watch_the_watcher(&mut kqueue_watcher);
                }

                event => warn!("Unknown event: {}", format!("{:?}", event).cyan()),
            }
        }
    }
}


// /// Hot reload configuration
// fn _handle_config_changes(log_level: &mut LevelFilter) {
//     let level = Config::load().get_log_level();
//     if level != *log_level {
//         info!("Changing log level to: {}", format!("{:?}", level).cyan());
//         *log_level = level
//     }
// }


/// Process file with event
fn process_file_event(
    abs_file_name: &str,
    kqueue_watcher: &mut Watcher,
    watched_file_states: &mut FileAndPosition,
    last_file: &mut String,
    config: &Config,
) {
    let file_path = Path::new(&abs_file_name);
    match metadata(file_path) {
        Ok(file_metadata) => {
            if file_metadata.is_dir() {
                trace!("{}: {}", "+DirLoad".magenta(), abs_file_name.cyan());
                walkdir_recursive(kqueue_watcher, file_path, config);
            } else {
                trace!("{}: {}", "+FileWatchHandle".magenta(), abs_file_name.cyan());
                calculate_position_and_handle(
                    file_metadata.len(),
                    watched_file_states,
                    &abs_file_name,
                    last_file,
                    &config,
                );
            }
        }

        Err(error_cause) => {
            // handle situation when logs are wiped out and unavailable to read anymore
            kqueue_watcher
                .remove_filename(file_path, EventFilter::EVFILT_VNODE)
                .map(|e| {
                    trace!("{}: {}", "-Watch".magenta(), abs_file_name.cyan());
                    e
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
                    walkdir_recursive(kqueue_watcher, file_path, config);
                } else if file_path.is_file() {
                    watch_file(kqueue_watcher, file_path);
                }
            } else {
                debug!(
                    "Dropped watch on file/dir: {}. Last value: {}. Error cause: {}",
                    format!("{:?}", &file_path).cyan(),
                    format!(
                        "{}",
                        watched_file_states
                            .remove(abs_file_name)
                            .unwrap_or_default()
                    )
                    .cyan(),
                    format!("{}", &error_cause).red()
                );
            }
        }
    };
    debug!(
        "Watched files list: [{}]",
        format!("{:?}", watched_file_states).cyan()
    );
}


/// Process file position and handle the event
fn calculate_position_and_handle(
    file_size: u64,
    watched_file_states: &mut FileAndPosition,
    abs_file_name: &str,
    last_file: &mut String,
    config: &Config,
) {
    let tail_bytes = config.tail_bytes.unwrap_or_default();
    let initial_file_position =
        if file_size + 1 > tail_bytes && !watched_file_states.contains_key(abs_file_name) {
            file_size - tail_bytes
        } else {
            *watched_file_states.get(abs_file_name).unwrap_or(&0)
        };
    if watched_file_states.contains_key(abs_file_name) {
        let current_position = *watched_file_states
            .get(abs_file_name)
            .unwrap_or(&initial_file_position);
        handle_file_event(current_position, file_size, abs_file_name, last_file);
        let _removed = watched_file_states
            .remove(abs_file_name)
            .unwrap_or_default();
        watched_file_states.insert(abs_file_name.to_string(), file_size);
    } else {
        watched_file_states.insert(abs_file_name.to_string(), initial_file_position);
        handle_file_event(initial_file_position, file_size, abs_file_name, last_file);
    }
}


/// Kqueue wrapper for watch()
fn watch_the_watcher(kqueue_watcher: &mut Watcher) {
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
fn watch_file(kqueue_watcher: &mut Watcher, file: &Path) {
    kqueue_watcher
        .remove_filename(file, EventFilter::EVFILT_VNODE)
        .map(|e| {
            trace!("{}: {}", "-Watch".magenta(), format!("{:?}", file).cyan());
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
                // | NOTE_ATTRIB
                // | NOTE_REVOKE,
        )
        .map(|e| {
            trace!("{}: {}", "+Watch".magenta(), format!("{:?}", file).cyan());
            e
        })
        .unwrap_or_else(|error_cause| {
            error!(
                "Could not watch file: {}. Caused by: {}",
                format!("{:?}", file).cyan(),
                error_cause.to_string().red()
            )
        });
}


/// Handle action triggered by an event
fn handle_file_event(
    file_position: u64,
    file_size: u64,
    file_path: &str,
    last_file: &mut String,
) {
    let watched_file = file_path.to_string();
    {
        debug!(
            "Watched file position: {}, file size: {}, file name: {}",
            format!("{}", file_position).cyan(),
            format!("{}", file_size).cyan(),
            watched_file.cyan()
        );
        trace!(
            "{}: {} {}",
            "+EventHandle".magenta(),
            watched_file.cyan(),
            format!("@{}", file_position).black()
        );

        // print header only when file is at beginning and not often than N bytes after previous one (limits header spam)
        if file_position == 0 || *last_file != watched_file {
            println!();
            println!(); // just start new entry after \n\n
            info!(
                "{} {}",
                watched_file.blue(),
                format!("@{}", file_position).black()
            );
        }

        // print content of file that triggered the event
        if file_position < file_size {
            let content = seek_file_to_position_and_read(&watched_file, file_position);
            println!("{}", content.join("\n"));
        }
    }

    *last_file = watched_file;
}


/// Set file position in bytes and print new file contents
fn seek_file_to_position_and_read(file_to_watch: &str, file_position: u64) -> Vec<String> {
    match File::open(&file_to_watch) {
        Ok(some_file) => {
            let mut cursor = BufReader::new(some_file);
            cursor.seek(SeekFrom::Start(file_position)).unwrap_or(0);
            let lines_out: Vec<_> = cursor.lines().filter_map(|line| line.ok()).collect();
            trace!("Lines out: '{}'", format!("{:?}", lines_out).cyan());
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
