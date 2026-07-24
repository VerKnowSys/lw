//! Configuration model and loading (RON-backed, with sane defaults).

use crate::consts::{
    DEFAULT_IGNORE_PATTERNS, DEFAULT_THEME, MAX_DIR_DEPTH, MAX_OPEN_FILES, STDOUT_DEV,
    TAIL_BYTES,
};
use crate::utils::write_append;
use std::{
    env,
    fs::read_to_string,
    io::{Error, ErrorKind},
    path::Path,
};

use colored::Colorize;
use log::LevelFilter;
use ron::ser::{PrettyConfig, to_string_pretty};
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

    /// syntect theme name for syntax-highlighted output (e.g.
    /// "base16-ocean.dark", "Solarized (dark)", "InspiredGitHub").
    #[serde(default = "default_theme")]
    pub theme: Option<String>,

    /// [`Self::ignore_patterns`] precompiled to char slices. Derived (never
    /// serialized): filled once by [`Config::with_compiled_globs`] so filename
    /// matching in the event hot path never re-parses the pattern strings.
    #[serde(skip)]
    ignore_globs: Vec<Vec<char>>,
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


/// Serde fallback for [`Config::theme`] so config files written before this
/// option existed still deserialize with the default theme.
fn default_theme() -> Option<String> {
    Some(DEFAULT_THEME.to_string())
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
            theme: default_theme(),
            ignore_globs: Vec::new(),
        }
        .with_compiled_globs()
    }
}


impl Config {
    /// Load the `lw` configuration file, falling back to defaults if it is
    /// missing or invalid.
    pub fn load() -> Config {
        let config = Config::get_or_create();
        read_to_string(&config)
            .and_then(|file_contents| {
                ron::from_str::<Config>(&file_contents).map_err(|err| {
                    let config_error = Error::new(ErrorKind::InvalidInput, err.to_string());
                    error!(
                        "Configuration error: {} in file: {}",
                        err.to_string().red(),
                        config.cyan()
                    );
                    config_error
                })
            })
            .map(Config::with_compiled_globs)
            .unwrap_or_default()
    }


    /// Precompile [`Self::ignore_patterns`] into `ignore_globs`. Runs after
    /// load/deserialization (which leaves the derived field empty) and inside
    /// [`Config::default`], so the compiled globs are always in sync.
    fn with_compiled_globs(mut self) -> Self {
        self.ignore_globs = self
            .ignore_patterns
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|pattern| pattern.chars().collect())
            .collect();
        self
    }


    /// Ignore patterns precompiled to char slices for matching (see
    /// [`Self::ignore_patterns`]).
    pub fn ignore_globs(&self) -> &[Vec<char>] {
        &self.ignore_globs
    }


    /// Candidate configuration file paths, in priority order.
    fn config_paths() -> [String; 3] {
        let home = env::var("HOME").unwrap_or_default();
        [
            format!("{home}/.lw.conf"),
            format!("{home}/.config/lw.conf"),
            String::from("lw.conf"),
        ]
    }

    /// Path of the first existing configuration file, if any.
    pub fn existing_config_path() -> Option<String> {
        Self::config_paths()
            .into_iter()
            .find(|path| Path::new(path).exists())
    }

    /// Path where a default configuration file is written when none exists.
    pub fn default_config_path() -> String {
        let [first, ..] = Self::config_paths();
        first
    }

    /// Return path of the config, writing a default one if none exists yet.
    pub fn get_or_create() -> String {
        Self::existing_config_path().unwrap_or_else(|| {
            let new_config_path = Self::default_config_path();
            write_append(
                &new_config_path,
                &to_string_pretty(
                    &Config::default(),
                    PrettyConfig::new().new_line("\n".to_string()),
                )
                .unwrap_or_default(),
            );
            new_config_path
        })
    }


    /// Get LevelFilter (log level) from configuration
    pub fn get_log_level(&self) -> LevelFilter {
        match self.log_level.as_deref().unwrap_or_default() {
            "OFF" => LevelFilter::Off,
            "ERROR" => LevelFilter::Error,
            "WARN" => LevelFilter::Warn,
            "DEBUG" => LevelFilter::Debug,
            "TRACE" => LevelFilter::Trace,
            _ => LevelFilter::Info,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::Config;
    use ron::ser::{PrettyConfig, to_string_pretty};

    /// The default config precompiles its ignore globs eagerly.
    #[test]
    fn default_has_compiled_ignore_globs() {
        let config = Config::default();
        assert_eq!(
            config.ignore_globs().len(),
            config.ignore_patterns.as_deref().unwrap_or_default().len()
        );
        assert!(!config.ignore_globs().is_empty());
    }

    /// The derived `ignore_globs` field is never written to the config file
    /// (only the human-editable `ignore_patterns` is).
    #[test]
    fn serialized_config_omits_derived_globs() {
        let ron = to_string_pretty(&Config::default(), PrettyConfig::new()).unwrap();
        assert!(!ron.contains("ignore_globs"), "derived field leaked: {ron}");
        assert!(ron.contains("ignore_patterns"));
    }

    /// A config parsed from RON (which leaves the derived field empty) has its
    /// globs recompiled by `with_compiled_globs`, matching its patterns.
    #[test]
    fn parsed_config_recompiles_globs() {
        let ron = to_string_pretty(&Config::default(), PrettyConfig::new()).unwrap();
        let parsed: Config = ron::from_str(&ron).unwrap();
        assert!(
            parsed.ignore_globs().is_empty(),
            "skipped field should deserialize empty"
        );
        let finalized = parsed.with_compiled_globs();
        assert_eq!(
            finalized.ignore_globs().len(),
            finalized.ignore_patterns.as_deref().unwrap_or_default().len()
        );
    }
}
