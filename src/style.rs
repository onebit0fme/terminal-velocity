//! Zero-dependency ANSI styling + terminal geometry. Both are emitted only for a
//! real terminal: color honors NO_COLOR / --no-color / CLICOLOR_FORCE, and the
//! board width adapts to the tty but falls back to a fixed value when piped — so
//! scripted output stays byte-deterministic, the same bargain color already makes.

use std::io::IsTerminal;

use crate::model::Tone;

/// Width used when output isn't a terminal (pipes, CI, tests) — keeps that path
/// deterministic. A conventional 80 columns.
const FALLBACK_WIDTH: usize = 80;
/// Detected widths are clamped here. The floor is the natural width of the densest
/// line (the cadence headline, the survival row); the hand-aligned art is tuned to
/// the low end. The cap keeps prose and rules from sprawling on ultra-wide screens.
const MIN_WIDTH: usize = 72;
const MAX_WIDTH: usize = 100;

#[derive(Clone, Copy)]
pub struct Palette {
    on: bool,
    pub width: usize,
}

impl Palette {
    /// `force_off` comes from `--no-color`.
    pub fn detect(force_off: bool) -> Self {
        let tty = std::io::stdout().is_terminal();
        let on = if force_off || std::env::var_os("NO_COLOR").is_some() {
            false
        } else {
            std::env::var_os("CLICOLOR_FORCE").is_some() || tty
        };
        Palette {
            on,
            width: detect_width(tty),
        }
    }

    /// A horizontal rule the full width of the board.
    pub fn rule(&self) -> String {
        "─".repeat(self.width)
    }

    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.wrap("31", s)
    }

    /// Color a string by a card's semantic tone.
    pub fn tone(&self, tone: Tone, s: &str) -> String {
        match tone {
            Tone::Calm => s.to_string(),
            Tone::Good => self.green(s),
            Tone::Watch => self.yellow(s),
            Tone::Alarm => self.red(s),
        }
    }
}

/// Board width: the live terminal width when attached to one, else a fixed
/// fallback (deterministic piped output). Clamped to a readable band.
fn detect_width(tty: bool) -> usize {
    if !tty {
        return FALLBACK_WIDTH;
    }
    stty_cols()
        .unwrap_or(FALLBACK_WIDTH)
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Columns from `stty size` ("rows cols"), read against the controlling terminal.
/// A shell-out, like the git plumbing — no crate, no ioctl FFI. `None` off-Unix or
/// when there's no tty to read (then the caller falls back).
fn stty_cols() -> Option<usize> {
    let tty = std::fs::File::open("/dev/tty").ok()?;
    let out = std::process::Command::new("stty")
        .arg("size")
        .stdin(tty)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
