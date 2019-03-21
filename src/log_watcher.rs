//! "lw" log-watcher utility

//! LW docs

#![deny(
    missing_docs,
    unstable_features,
    unsafe_code,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications
)]


#[macro_use]
extern crate log;
extern crate kqueue_sys;

use kqueue_sys::*;
use kqueue::*;
use kqueue::Ident::*;
use std::io::prelude::*;
use std::io::{SeekFrom, BufReader};
use std::fs::File;
use std::fs::metadata;
use std::collections::HashMap;

use std::env;
use std::path::Path;
use walkdir::WalkDir;
use std::process::exit;
use std::fmt::Display;
use chrono::Local;
use colored::Colorize;
use log::LevelFilter;
use fern::Dispatch;


/// FileAndPosition alias type for list of tuples of File path and Cursor positions
type FileAndPosition = HashMap<String, u64>;

const STDOUT_DEV: &str = "/dev/stdout";
const MIN_DIR_DEPTH: usize = 1;
const MAX_DIR_DEPTH: usize = 3;


fn fatal<S: Display>(fmt: S) -> ! {
    error!("ERROR: {}", fmt.to_string().red());
    exit(1)
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
        .chain(
            File::open(STDOUT_DEV)
                .unwrap_or_else(|_|
                    fatal(format!("{}: STDOUT device {} is not available! Something is terribly wrong here!",
                             "FATAL ERROR".red(), STDOUT_DEV.yellow()))
                )
        )
        .apply()
        .unwrap_or_else(|err| {
            fatal(format!("{}: Couldn't initialize Log-Watcher. Details: {}",
                   "FATAL ERROR".red(), err.to_string().yellow()));
        });

    // mutable hashmap keeping position of all watched files:
    let mut watched_file_states = FileAndPosition::new();

    // mutable kqueue watcher:
    let mut kqueue_watcher
        = Watcher::new()
            .unwrap_or_else(|e| fatal(format!("Could not create kq watcher: {}", e)));

    // read paths given as arguments:
    let paths_to_watch: Vec<String>
        = env::args()
            .skip(1) // first arg is $0
            .collect();

    if paths_to_watch.is_empty() {
        fatal("No paths specified as arguments! You have to specify at least a single file or directory to watch!");
    }

    debug!("Watching paths: {}", paths_to_watch.join(", "));
    paths_to_watch
        .iter()
        .for_each(|a_path| // Resursively filter out all unreadable/unaccessible/inproper files:
            WalkDir::new(Path::new(&a_path))
                .follow_links(true)
                .min_depth(MIN_DIR_DEPTH)
                .max_depth(MAX_DIR_DEPTH)
                .into_iter()
                .filter_map(|element| element.ok())
                .for_each(|element| watch_file(&mut kqueue_watcher, element.path()))
        );

    // Watch all paths:
    kqueue_watcher
        .watch()
        .unwrap_or_else(|error_cause| fatal(format!("kqueue failed: {}", error_cause)));

    // handle events:
    kqueue_watcher
        .iter()
        .for_each(|kqueue_event| {
            match kqueue_event.ident {
                Filename(_file_descriptor, abs_file_name) =>
                    handle_file_event(&mut watched_file_states, &abs_file_name),

                Fd(file_descriptor) =>
                    debug!("New event: FD: {}", file_descriptor),

                Pid(pid) =>
                    debug!("New event: PID: {}", pid),

                Signal(signal) =>
                    debug!("New event: SIGNAL: {}", signal),

                Timer(time) =>
                    debug!("New event: TIMER: {}", time),
            }
        })
}


fn watch_file(kqueue_watcher: &mut Watcher, file: &Path) {
    kqueue_watcher
        .add_filename(
            &file,
            EventFilter::EVFILT_VNODE, // NOTE: no NOTE_TRUNCATE on Darwin + ignore on NOTE_ATTRIB and NOTE_EXTEND
            NOTE_DELETE | NOTE_WRITE | NOTE_LINK | NOTE_RENAME | NOTE_REVOKE
        )
        .unwrap_or_else(|error_cause| fatal(format!("Could not watch file {:?}: {}", file, error_cause)));
}


fn handle_file_event(states: &mut FileAndPosition, file_path: &str) {
    let file_entry_in_hashmap
        = states
            .iter()
            .find(|hashmap| *hashmap.0 == file_path);

    match file_entry_in_hashmap {
        Some((watched_file, file_position)) => {
            let file_size = match metadata(&watched_file) {
                Ok(file_metadata) => file_metadata.len(),
                Err(_) => 0,
            };
            if *file_position < file_size {
                seek_file_to_position_and_print(&watched_file, *file_position);
                states
                    .insert(file_path.to_string(), file_size);
            }
        },

        None => {
            states
                .insert(file_path.to_string(), 0);
        }
    }
}


fn seek_file_to_position_and_print(file_to_watch: &str, file_position: u64) {
    match File::open(&file_to_watch) {
        Ok(some_file) => {
            let mut cursor = BufReader::new(some_file);
            cursor
                .seek(SeekFrom::Start(file_position))
                .unwrap_or_else(|_| 0);
            println!(); // just start new entry from \n
            info!("{}", file_to_watch.blue());
            let content: Vec<String>
                = cursor
                    .lines()
                    .filter_map(|line| line.ok())
                    .collect();
            println!("{}", content.join("\n"));
        },

        Err(error_cause) =>
            error!("Couldn't open file: {}. Error cause: {}",
                   file_to_watch.yellow(), error_cause.to_string().red()),
    }
}
