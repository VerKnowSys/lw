use std::{
    env,
    fs::{read_to_string, OpenOptions},
    io::{Error, ErrorKind, Write},
    path::Path,
};

use colored::Colorize;
use log::LevelFilter;
use nanoserde::{DeRon, SerRon};


/// Defines stdout file
const STDOUT_DEV: &str = "/dev/stdout";

/// Maximum directory depth to watch
const MAX_DIR_DEPTH: usize = 5;

/// Maximum watched files
const MAX_OPEN_FILES: usize = 1023;

/// Read tail of this length from large files
const TAIL_BYTES: u64 = 2048;


#[derive(Clone, Debug, DeRon, SerRon)]
pub struct Config {
    /// Print output. Default is /dev/stdout
    pub output: Option<String>,

    /// Log level
    pub log_level: Option<String>,

    /// Max amount of open files by watcher
    pub max_open_files: Option<usize>,

    /// How many bytes of tail to show fornewly watched files
    pub tail_bytes: Option<u64>,

    /// Follow symlinks?
    pub follow_links: Option<bool>,

    /// How deep to go in directory tree
    pub max_dir_depth: Option<usize>,
}


impl Default for Config {
    fn default() -> Self {
        Config {
            output: Some(String::from(STDOUT_DEV)),
            log_level: Some(String::from("INFO")),
            max_open_files: Some(MAX_OPEN_FILES),
            tail_bytes: Some(TAIL_BYTES),
            max_dir_depth: Some(MAX_DIR_DEPTH),
            follow_links: Some(true),
        }
    }
}


/// Write-once-and-atomic to a file
pub fn write_append(file_path: &str, contents: &str) {
    if !contents.is_empty() {
        let mut options = OpenOptions::new();
        match options.create(true).append(true).open(&file_path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes()).unwrap_or_else(|_| {
                    panic!("Access denied? File can't be written: {}", &file_path)
                });
                debug!("Atomically written data to file: {}", &file_path);
            }

            Err(err) => {
                error!(
                    "Atomic write to: {} has failed! Cause: {}",
                    &file_path,
                    err.to_string()
                )
            }
        }
    }
}


impl Config {
    /// Load Krecik configuration file
    pub fn load() -> Config {
        let config_paths = [
            &format!("{}/.lw.conf", env::var("HOME").unwrap_or_default()),
            &format!("{}/.config/lw.conf", env::var("HOME").unwrap_or_default()),
            "/Services/Lw/service.conf",
            "lw.conf",
        ];
        let config: String = config_paths
            .iter()
            .filter(|file| Path::new(file).exists())
            .take(1)
            .cloned()
            .collect();
        if config.is_empty() {
            let first_conf = config_paths[0];
            let new_conf = Config::default();
            write_append(
                first_conf,
                &format!("{}\n", SerRon::serialize_ron(&new_conf)),
            )
        }
        debug!("Reading config: {}", config.cyan());
        read_to_string(&config)
            .and_then(|file_contents| {
                DeRon::deserialize_ron(&*file_contents).map_err(|err| {
                    let config_error = Error::new(ErrorKind::InvalidInput, err.to_string());
                    error!(
                        "Configuration error: {} in file: {}",
                        err.to_string().red(),
                        config.cyan()
                    );
                    config_error
                })
            })
            .unwrap_or_default()
    }


    /// Get LevelFilter (log level) from configuration
    pub fn get_log_level(&self) -> LevelFilter {
        let level = self.log_level.clone().unwrap_or_default();
        match &level[..] {
            "OFF" => LevelFilter::Off,
            "ERROR" => LevelFilter::Error,
            "WARN" => LevelFilter::Warn,
            "INFO" => LevelFilter::Info,
            "DEBUG" => LevelFilter::Debug,
            "TRACE" => LevelFilter::Trace,
            _ => LevelFilter::Info,
        }
    }
}
