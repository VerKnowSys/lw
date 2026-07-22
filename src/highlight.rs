//! Syntax highlighting for watched-file output, backed by `syntect`.
//!
//! A single [`Highlighter`] owns the (expensive to build) syntax and theme
//! sets and is created once via [`highlighter`]. The actual per-line
//! highlighting in [`Highlighter::highlight`] is a pure transformation from
//! `(extension, lines)` to ANSI-escaped lines.

use crate::consts::DEFAULT_THEME;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

/// ANSI reset appended after each highlighted line so colors don't bleed.
const ANSI_RESET: &str = "\x1b[0m";

/// Process-wide highlighter, initialised on first use.
static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();

/// Initialise the shared highlighter with the given theme. Only the first call
/// takes effect, so call this once at startup before any highlighting happens.
pub fn init(theme_name: &str) {
    let _ = HIGHLIGHTER.set(Highlighter::new(theme_name));
}

/// Return the shared [`Highlighter`], building it with the default theme if it
/// has not been [`init`]ialised yet.
pub fn highlighter() -> &'static Highlighter {
    HIGHLIGHTER.get_or_init(|| Highlighter::new(DEFAULT_THEME))
}


/// Owns the syntax definitions and the color theme used for highlighting.
#[derive(Debug)]
pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    /// Build a highlighter from syntect's bundled defaults, using `theme_name`
    /// (falling back to a known-present theme if it is missing).
    pub fn new(theme_name: &str) -> Self {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let mut themes = ThemeSet::load_defaults().themes;
        let theme = themes
            .remove(theme_name)
            .or_else(|| themes.remove("base16-ocean.dark"))
            .or_else(|| themes.values().next().cloned())
            .unwrap_or_default();
        Highlighter {
            syntaxes,
            theme,
        }
    }

    /// Highlight `lines` as source of the language matching `extension`,
    /// returning ANSI-escaped strings. Unknown extensions fall back to plain
    /// text, and any per-line highlighting error yields the original line.
    pub fn highlight(&self, extension: &str, lines: &[String]) -> Vec<String> {
        let syntax = self
            .syntaxes
            .find_syntax_by_extension(extension)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        lines
            .iter()
            .map(|line| {
                match highlighter.highlight_line(line, &self.syntaxes) {
                    Ok(ranges) => {
                        format!(
                            "{}{}",
                            as_24_bit_terminal_escaped(&ranges, false),
                            ANSI_RESET
                        )
                    }
                    Err(_) => line.clone(),
                }
            })
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use super::Highlighter;

    #[test]
    fn known_extension_emits_ansi_color_codes() {
        let highlighter = Highlighter::new("base16-ocean.dark");
        let out = highlighter.highlight("rs", &["fn main() {}".to_string()]);
        assert_eq!(out.len(), 1);
        // Rust source should be colored -> contains ANSI escape sequences.
        assert!(
            out[0].contains('\x1b'),
            "expected ANSI codes, got {:?}",
            out[0]
        );
        assert!(out[0].contains("main"));
    }

    #[test]
    fn unknown_extension_falls_back_to_plain_text() {
        let highlighter = Highlighter::new("base16-ocean.dark");
        // Plain text still round-trips the content (possibly with color codes),
        // and must preserve the original text.
        let out =
            highlighter.highlight("this-ext-does-not-exist", &["hello world".to_string()]);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("hello world"));
    }

    #[test]
    fn line_count_is_preserved() {
        let highlighter = Highlighter::new("base16-ocean.dark");
        let lines = vec!["let x = 1;".to_string(), "let y = 2;".to_string()];
        assert_eq!(highlighter.highlight("rs", &lines).len(), 2);
    }
}
