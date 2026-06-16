//! Core domain + presentation types.

/// What kind of work a commit represents. Heuristic today (see `intent`);
/// the swap-point for a learned classifier (an external API call) later.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Intent {
    Feature,
    Refactor,
    Fix,
    Web,
    Docs,
    Test,
    Ops,
    Revert,
    Other,
}

impl Intent {
    pub fn label(self) -> &'static str {
        match self {
            Intent::Feature => "feature",
            Intent::Refactor => "refactor",
            Intent::Fix => "fix",
            Intent::Web => "web",
            Intent::Docs => "docs",
            Intent::Test => "test",
            Intent::Ops => "ops",
            Intent::Revert => "revert",
            Intent::Other => "other",
        }
    }

    /// Inverse of [`label`], for the blame cache. Unknown → `Other` (fail-soft on a
    /// cache written by a future version that added an intent).
    pub fn from_label(s: &str) -> Intent {
        match s {
            "feature" => Intent::Feature,
            "refactor" => Intent::Refactor,
            "fix" => Intent::Fix,
            "web" => Intent::Web,
            "docs" => Intent::Docs,
            "test" => Intent::Test,
            "ops" => Intent::Ops,
            "revert" => Intent::Revert,
            _ => Intent::Other,
        }
    }
}

/// One non-merge commit, as read from `git log --numstat`.
#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub ts: i64, // committer time, unix epoch seconds
    pub subject: String,
    pub author_email: String,
    pub author_name: String,
    pub added: i64,
    pub deleted: i64,
    pub files: Vec<String>,
    pub intent: Intent,
}

impl Commit {
    pub fn churn(&self) -> i64 {
        self.added + self.deleted
    }

    /// Whether this commit is the running user's (see [`Me::matches`]).
    pub fn by(&self, me: &Me) -> bool {
        me.matches(&self.author_email, &self.author_name)
    }
}

/// The running user's git identity, for `--me`. Self-instrumentation only — there
/// is deliberately no `--author=<someone-else>` (that would be the surveillance line).
#[derive(Clone, Debug)]
pub struct Me {
    pub email: String,
    pub name: String,
}

impl Me {
    /// Does an author identity match the running user? Email OR (name, when set),
    /// case-insensitive. Name is a fallback so email drift (work laptop, noreply
    /// addresses) still matches. The single definition of "me", shared by the
    /// commit filter (`Commit::by`) and the survival mask (`Collection::author_mask`).
    pub fn matches(&self, email: &str, name: &str) -> bool {
        email.eq_ignore_ascii_case(&self.email)
            || (!self.name.is_empty() && name.eq_ignore_ascii_case(&self.name))
    }
}

/// Semantic severity of a card, independent of the state word (e.g. batch
/// "easing" is Good while "rising" is Watch). Drives color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Calm,  // nothing to see (default fg)
    Good,  // explicit reassurance (green)
    Watch, // drifting; look (yellow)
    Alarm, // act (red)
}

impl Tone {
    pub fn rank(self) -> u8 {
        match self {
            Tone::Calm => 0,
            Tone::Good => 1,
            Tone::Watch => 2,
            Tone::Alarm => 3,
        }
    }

    /// The status glyph shown in the cockpit's left gutter. Symbol-distinct (not
    /// only color) so the board still reads under NO_COLOR and for colorblind eyes
    /// — color is reinforcement, not the channel. This is the whole "verdict": a
    /// column of these, git-status-style, that the reader triages.
    pub fn glyph(self) -> &'static str {
        match self {
            Tone::Calm => "·",
            Tone::Good => "✓",
            Tone::Watch => "▲",
            Tone::Alarm => "■",
        }
    }
}

/// One leading-indicator row in the cockpit: a status glyph (its `tone`) +
/// headline + sparkline + where-you-sit + a plain state word, and either an
/// action or an explicit permission to ignore.
#[derive(Clone, Debug)]
pub struct Card {
    pub key: String,
    pub headline: String,
    pub spark: String,
    /// Raw weekly values behind the sparkline (oldest→newest) — lets the HTML
    /// report draw real bars instead of unicode blocks.
    pub spark_values: Vec<f64>,
    /// One-word state: steady / rising / low / healthy / ...
    pub state: String,
    pub tone: Tone,
    /// The one action, OR the explicit "ignore" (both are first-class).
    pub note: Option<String>,
    /// Extra context shown only under `--explain` (e.g. thrash's intent breakdown).
    pub detail: Option<String>,
    /// False when the metric needs the full blame pass we haven't run yet.
    pub available: bool,
}

/// One repo's survival curve, surfaced as the foundation the other stats rest on.
#[derive(Clone, Debug)]
pub struct RepoSurvival {
    /// Repo label (shown only when several are aggregated).
    pub label: String,
    /// S(age) sampled over line age (oldest→newest age), for a sparkline/SVG.
    pub curve: Vec<f64>,
    /// Half-life, compact: "~491c / ~86d" or "not reached".
    pub half_life: String,
    /// % of analyzed lines still alive at HEAD (the right-censored fraction).
    pub alive_pct: f64,
}

/// Weekday × hour commit punchcard — the cadence drill-down. `counts[day][hour]`,
/// day 0 = Monday, hour 0..23 in local time.
#[derive(Clone, Debug)]
pub struct Heatmap {
    pub counts: Vec<Vec<u32>>, // [7][24] raw commit counts — the all-time rhythm (shading + peak)
    pub max: u32,              // busiest cell (rhythm shading denominator)
    pub total: u32,
    pub peak_day: usize,
    pub peak_hour: usize,
    pub tz: String,
    /// Per-weekday `[7]` and per-hour `[24]` *shift*: the standardized residual of recent
    /// (recency-weighted) activity vs that margin's own all-time rate,
    /// z = (recent − expected)/√expected. > 0 = that day / that hour is heating (busier
    /// lately than its usual share), < 0 = cooling, ~0 = steady. Marked on the grid's axes,
    /// not a second grid — a whole-day / whole-hour margin is a far larger, more stable
    /// sample than any single cell, so it reports a real trend, not cell noise. ±4σ clamp.
    pub day_shift: Vec<f64>,
    pub hour_shift: Vec<f64>,
    /// Weekend/night commit share — all-time (the rhythm) and recency-weighted "lately"
    /// (the recent vector, reconciles with the cockpit cadence card). Shown as `a→b`.
    pub weekend_all: f64,
    pub night_all: f64,
    pub weekend_lately: f64,
    pub night_lately: f64,
}

/// Weeks of history under which trend metrics are too thin to trust; below it
/// the board is flagged "provisional". One tunable constant, beside the data it
/// gates (rather than scattered through the render layer).
const BASELINE_WEEKS: usize = 3;

/// The whole one-screen cockpit: a status-board of cards, with the survival
/// foundation below them. No composed verdict — the reader triages the glyphs.
#[derive(Clone, Debug)]
pub struct Cockpit {
    pub branch: String,
    pub window: String,
    /// `--at`: date of the anchor (the window's upper edge); `None` at HEAD. Lets
    /// the report frame its drill-downs as "… to <date>" instead of implying now.
    pub as_of: Option<String>,
    /// The survival curve(s) — S(age) — that weight thrash/excision. Shown below
    /// the indicators, without a status glyph, to mark it as the foundation the
    /// graded metrics rest on rather than a graded metric itself.
    pub survival: Vec<RepoSurvival>,
    /// `--me`: the board is scoped to the running user (changes survival wording).
    pub personal: bool,
    pub cards: Vec<Card>,
    pub footer: String,
    /// Weeks of history available; drives the coverage-honesty rule.
    pub coverage_weeks: usize,
}

impl Cockpit {
    /// Too little history to trust the trends? Both surfaces flag the board
    /// "provisional" when so; each formats that caveat in its own idiom.
    pub fn is_provisional(&self) -> bool {
        self.coverage_weeks < BASELINE_WEEKS
    }
}
