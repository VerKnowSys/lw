//! "lw" log-watcher utility — binary entry point.

#![forbid(unsafe_code)]
#![deny(
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications,
    bad_style,
    dead_code,
    unused,
    while_true
)]

/// Use MiMalloc as default allocator:
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


#[macro_use]
extern crate log;

use chrono::Local;
use colored::Colorize;
use fern::Dispatch;
use kqueue2::{Ident::*, Watcher};
use lw::config::Config;
use lw::consts::DEFAULT_THEME;
use lw::types::FileAndPosition;
use lw::utils::{process_file_event, walkdir_recursive, watch_the_watcher};
use std::{env, fs::OpenOptions, path::Path, process::exit, thread, time::Duration};


fn main() {
    // Note whether a config existed *before* loading, since load() creates a
    // default one when missing. We report it once the logger is initialised.
    let wrote_default_config = Config::existing_config_path().is_none();
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
    let mut kqueue_watcher = Watcher::new().expect("Could not create kqueue watcher!");

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
        .chain(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&output)
                .unwrap_or_else(|_| {
                    panic!("{}: Couldn't open: {}!", "FATAL ERROR".red(), output.cyan())
                }),
        )
        .apply()
        .expect("Couldn't initialize Fern logger!");

    if wrote_default_config {
        info!(
            "No configuration file found — wrote defaults to: {}",
            Config::default_config_path().cyan()
        );
    }

    // Initialise syntax highlighting with the configured theme.
    lw::highlight::init(config.theme.as_deref().unwrap_or(DEFAULT_THEME));

    debug!("Watching paths: {}", paths_to_watch.join(", "));
    if paths_to_watch.is_empty() {
        error!("FATAL ERROR: {}", "No paths specified as arguments! You have to specify at least a single directory/file to watch!".red());
        exit(1)
    }

    // initial watches for specified dirs/files:
    paths_to_watch.into_iter().for_each(|a_path| {
        // Handle case when given a file as argument
        walkdir_recursive(
            &mut kqueue_watcher,
            &mut watched_file_states,
            &mut last_file,
            Path::new(&a_path),
            &config,
        );
    });

    // handle events dynamically, including new files
    loop {
        watch_the_watcher(&mut kqueue_watcher);
        while let Some(an_event) = kqueue_watcher.iter().next() {
            debug!("Watched files: {}", watched_file_states.len());
            match an_event.ident {
                Filename(_file_descriptor, abs_file_name) => {
                    process_file_event(
                        &abs_file_name,
                        &mut kqueue_watcher,
                        &mut watched_file_states,
                        &mut last_file,
                        &config,
                    );
                    watch_the_watcher(&mut kqueue_watcher);
                }

                event => warn!("Unknown event: {}", format!("{event:?}").cyan()),
            }
        }

        // throttle 100ms
        thread::sleep(Duration::from_millis(100));
    }
}
