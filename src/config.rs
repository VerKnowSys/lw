//! Configuration model and loading (RON-backed, with sane defaults).

use crate::consts::{DEFAULT_IGNORE_PATTERNS, MAX_DIR_DEPTH, MAX_OPEN_FILES, STDOUT_DEV, TAIL_BYTES};
use crate::utils::write_append;
use std::{
    env,
    fs::read_to_string,
    io::{Error, ErrorKind},
    path::Path,
};

use colored::Colorize;
use log::LevelFilter;
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};


/// Runtime configuration for the log watcher.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    /// Filename glob patterns to ignore (transient temp/swap/backup files).
    /// Missing from older config files -> falls back to the built-in defaults.
    #[serde(default = "default_ignore_patterns")]
    pub ignore_patterns: Option<Vec<String>>,
}


/// Serde fallback for [`Config::ignore_patterns`] so config files written
/// before this option existed still deserialize with sensible defaults.
fn default_ignore_patterns() -> Option<Vec<String>> {
    Some(
        DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|pattern| pattern.to_string())
            .collect(),
    )
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
            ignore_patterns: default_ignore_patterns(),
        }
    }
}


impl Config {
    /// Load Krecik configuration file
    pub fn load() -> Config {
        let config = Config::get_or_create();
        read_to_string(&config)
            .and_then(|file_contents| {
                ron::from_str(&*file_contents).map_err(|err| {
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


    /// Return path of the config
    pub fn get_or_create() -> String {
        let config_paths = [
            &format!("{}/.lw.conf", env::var("HOME").unwrap_or_default()),
            &format!("{}/.config/lw.conf", env::var("HOME").unwrap_or_default()),
            "lw.conf",
        ];
        config_paths
            .iter()
            .find(|file| Path::new(file).exists())
            .map(|found| found.to_string())
            .unwrap_or_else(|| {
                let first_conf = config_paths[0].to_string();
                write_append(
                    &first_conf,
                    &to_string_pretty(
                        &Config::default(),
                        PrettyConfig::new().new_line("\n".to_string()),
                    )
                    .unwrap_or_default(),
                );
                first_conf
            })
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
