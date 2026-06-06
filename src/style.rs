//! Zero-dependency ANSI styling. Color is emitted only to a real terminal,
//! and honors NO_COLOR / --no-color / CLICOLOR_FORCE (the de-facto standards).

use std::io::IsTerminal;

use crate::model::Tone;

#[derive(Clone, Copy)]
pub struct Palette {
    on: bool,
}

impl Palette {
    /// `force_off` comes from `--no-color`.
    pub fn detect(force_off: bool) -> Self {
        if force_off || std::env::var_os("NO_COLOR").is_some() {
            return Palette { on: false };
        }
        let forced = std::env::var_os("CLICOLOR_FORCE").is_some();
        Palette {
            on: forced || std::io::stdout().is_terminal(),
        }
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

    /// Color the verdict by the worst tone on the board (the cockpit's mood).
    pub fn mood(&self, worst: Tone, s: &str) -> String {
        let body = match worst {
            Tone::Alarm => self.red(s),
            Tone::Watch => self.yellow(s),
            _ => self.green(s),
        };
        self.bold(&body)
    }
}
