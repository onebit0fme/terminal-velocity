//! Core domain + presentation types.

/// What kind of work a commit represents. Heuristic today (see `intent`);
/// the swap-point for a learned classifier (an external API call) later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
}

/// One non-merge commit, as read from `git log --numstat`.
#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub ts: i64, // committer time, unix epoch seconds
    pub subject: String,
    pub added: i64,
    pub deleted: i64,
    pub files: Vec<String>,
    pub intent: Intent,
}

impl Commit {
    pub fn churn(&self) -> i64 {
        self.added + self.deleted
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
}

/// One leading-indicator row in the cockpit: headline + sparkline + where-you-sit
/// + a plain verdict, and either an action or an explicit permission to ignore.
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

/// The whole one-screen cockpit. Verdict first; cards are the drill-down.
#[derive(Clone, Debug)]
pub struct Cockpit {
    pub branch: String,
    pub window: String,
    pub verdict: String,
    /// The survival curve(s) — S(age) — that weight thrash/excision. Shown right
    /// under the verdict so the rest of the board has a foundation.
    pub survival: Vec<RepoSurvival>,
    pub cards: Vec<Card>,
    pub footer: String,
    /// Weeks of history available; drives the coverage-honesty rule.
    pub coverage_weeks: usize,
}
