//! terminal velocity (`tv`) — a git-status for build-flow health.
//!
//! Cockpit-first: a plain-English verdict, then 3-4 leading indicators, each as
//! headline + sparkline + where-you-sit-vs-your-own-history + an action or an
//! explicit "ignore". Mostly quiet; surfaces the one or two things drifting.
//!
//! Scaffold status: batch / cadence / net / intent are live (numstat only).
//! Survival-weighted flow / thrash / excision are wired as pending until the
//! blame-at-death + incremental-cache pass lands (see src/git.rs).

#![allow(dead_code)] // scaffold: survival/* and some helpers aren't wired yet

mod git;
mod intent;
mod metrics;
mod model;
mod render;
mod spark;
mod style;
mod survival;
mod verdict;

use std::collections::HashMap;
use std::path::Path;
use std::process::exit;

enum Command {
    Status,
    Thrash,
    Hotspots,
    Cadence,
    Report,
    Explain,
}

struct Config {
    repos: Vec<String>,
    command: Command,
    report: bool,
    color_off: bool,
    all_time: bool,
}

/// Default window for the drill-down commands — matches the cockpit's trailing
/// 8 weeks, anchored to the latest commit. `--all` widens to full history.
const WINDOW_SECS: i64 = 8 * 7 * 86_400;

/// Newest commit across all repos — the shared window anchor when aggregating.
fn latest_ts_across(repos: &[String]) -> Result<i64, String> {
    let mut latest: Option<i64> = None;
    for r in repos {
        let ts = git::latest_ts(r).map_err(|e| format!("{r}: {e}"))?;
        latest = Some(latest.map_or(ts, |m| m.max(ts)));
    }
    latest.ok_or_else(|| "no repositories given".to_string())
}

/// Header label: the branch for a single repo, repo names (or a count) when
/// several are aggregated.
fn scope_label(repos: &[String]) -> String {
    if repos.len() == 1 {
        return git::current_branch(&repos[0]).unwrap_or_else(|_| "HEAD".to_string());
    }
    let names = repo_labels(repos);
    if names.len() <= 3 {
        names.join(" + ")
    } else {
        format!("{} repos", names.len())
    }
}

// Worktree dirs whose own name isn't a useful label — fall back to the project
// dir that contains them (…/my-project/main -> "my-project").
const WORKTREE_DIRS: &[&str] = &["main", "master", "trunk", "develop", "default"];

/// A short, human label for a repo path, aware of git-worktree layouts.
fn repo_label(path: &str) -> String {
    let p = Path::new(path);
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if base.is_empty() || base == "." {
        if let Ok(abs) = std::fs::canonicalize(path) {
            if let Some(n) = abs.file_name().and_then(|s| s.to_str()) {
                return n.to_string();
            }
        }
        return "repo".to_string();
    }
    if WORKTREE_DIRS.contains(&base) {
        if let Some(parent) = p
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
        {
            if !parent.is_empty() && parent != "." {
                return parent.to_string();
            }
        }
    }
    base.to_string()
}

/// Labels for a set of repos, disambiguating collisions with a parent segment.
fn repo_labels(repos: &[String]) -> Vec<String> {
    let base: Vec<String> = repos.iter().map(|r| repo_label(r)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for b in &base {
        *counts.entry(b.as_str()).or_insert(0) += 1;
    }
    base.iter()
        .enumerate()
        .map(|(i, b)| {
            if counts[b.as_str()] <= 1 {
                return b.clone();
            }
            let parent = Path::new(&repos[i])
                .parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if parent.is_empty() {
                b.clone()
            } else {
                format!("{parent}/{b}")
            }
        })
        .collect()
}

fn parse_args() -> Result<Config, String> {
    let mut repos: Vec<String> = Vec::new();
    let mut command = Command::Status;
    let mut report = false;
    let mut color_off = false;
    let mut all_time = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_help();
                exit(0);
            }
            "-V" | "--version" => {
                println!("tv (terminal-velocity) {}", env!("CARGO_PKG_VERSION"));
                exit(0);
            }
            "--report" => report = true,
            "--no-color" => color_off = true,
            "--all" => all_time = true,
            "--repo" => repos.push(it.next().ok_or("--repo requires a path")?),
            "status" => command = Command::Status,
            "thrash" => command = Command::Thrash,
            "hotspots" => command = Command::Hotspots,
            "cadence" => command = Command::Cadence,
            "report" => command = Command::Report,
            "explain" | "tree" => command = Command::Explain,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if repos.is_empty() {
        repos.push(".".to_string());
    }
    Ok(Config {
        repos,
        command,
        report,
        color_off,
        all_time,
    })
}

fn print_help() {
    print!(
        "\
terminal velocity (tv) — is your build's speed real throughput, or just thrashing?

USAGE:
    tv [COMMAND] [--repo <path>]... [--report]

COMMANDS:
    status      one-screen build-flow cockpit (default)
    thrash      in-place rewrite (S-weighted) ranked by directory
    hotspots    files ranked by churn × complexity
    cadence     weekday × hour commit punchcard (when commits land)
    report      write every view to one self-contained HTML page (tv-report.html)
    explain     print the heuristic decision tree

OPTIONS:
    --repo <path>   analyze this repo; repeat the flag to aggregate several
                    repos into one combined history (default: current directory)
    --report        shorthand for the `report` command (works with any command)
    --no-color      disable color (also respects NO_COLOR)
    --all           thrash/hotspots over all history (default: last 8 weeks)
    -h, --help      show this help
    -V, --version   show version
"
    );
}

/// Per-repo (label, change-frequency, complexity) — the hotspots inputs.
fn repo_files(
    repos: &[String],
    labels: &[String],
    since: Option<i64>,
) -> Result<Vec<metrics::RepoFiles>, String> {
    let mut out = Vec::new();
    for (repo, label) in repos.iter().zip(labels) {
        let f = git::file_change_freq(repo, since).map_err(|e| format!("{repo}: {e}"))?;
        let c = git::file_complexity(repo).map_err(|e| format!("{repo}: {e}"))?;
        out.push((label.clone(), f, c));
    }
    Ok(out)
}

/// Per-repo (label, churn-by-path) — the denominator for the thrash %.
fn repo_churn(
    repos: &[String],
    labels: &[String],
    since: Option<i64>,
) -> Result<Vec<metrics::RepoChurn>, String> {
    let mut out = Vec::new();
    for (repo, label) in repos.iter().zip(labels) {
        let c = git::file_churn_totals(repo, since).map_err(|e| format!("{repo}: {e}"))?;
        out.push((label.clone(), c));
    }
    Ok(out)
}

fn run(cfg: Config) -> Result<(), String> {
    let palette = style::Palette::detect(cfg.color_off);
    let multi = cfg.repos.len() > 1;
    let labels = repo_labels(&cfg.repos);
    // `report` command and the `--report` flag are the same thing: the full page.
    let want_report = cfg.report || matches!(cfg.command, Command::Report);

    match cfg.command {
        Command::Explain => {
            render::print_explain(&palette);
            return Ok(());
        }
        // Hotspots to the terminal needs no blame pass; the report does, so it
        // falls through to the full pipeline below.
        Command::Hotspots if !want_report => {
            let since = if cfg.all_time {
                None
            } else {
                Some(latest_ts_across(&cfg.repos)? - WINDOW_SECS)
            };
            let per_repo = repo_files(&cfg.repos, &labels, since)?;
            let rows = metrics::hotspots(&per_repo, 12);
            render::print_hotspots(&rows, since.is_some(), multi, &palette);
            return Ok(());
        }
        // Cadence is a commit-time punchcard — commits only, no blame pass.
        Command::Cadence if !want_report => {
            let mut commits: Vec<model::Commit> = Vec::new();
            for repo in &cfg.repos {
                commits.extend(git::load_commits(repo).map_err(|e| format!("{repo}: {e}"))?);
            }
            if commits.is_empty() {
                return Err("no non-merge commits found across the given repo(s)".into());
            }
            let heat = metrics::cadence_heatmap(&commits);
            render::print_heatmap(&heat, &scope_label(&cfg.repos), &palette);
            return Ok(());
        }
        _ => {}
    }

    // Status / Thrash / Report all need the per-repo survival collections (S is
    // fit per-repo — repo frailty differs — and output is attributed by repo).
    let mut commits: Vec<model::Commit> = Vec::new();
    let mut repos: Vec<(String, git::Collection)> = Vec::new();
    for (repo, label) in cfg.repos.iter().zip(&labels) {
        let cs = git::load_commits(repo).map_err(|e| format!("{repo}: {e}"))?;
        if cs.is_empty() {
            continue; // an empty repo in a multi-repo set just contributes nothing
        }
        let col = git::collect_cached(repo, &cs).map_err(|e| format!("{repo}: {e}"))?;
        repos.push((label.clone(), col));
        commits.extend(cs);
    }
    if commits.is_empty() {
        return Err("no non-merge commits found across the given repo(s)".into());
    }
    let header = scope_label(&cfg.repos);

    let anchor = commits.iter().map(|c| c.ts).max().unwrap_or(0);
    let (since, recent_cut) = if cfg.all_time {
        (None, None)
    } else {
        (Some(anchor - WINDOW_SECS), Some(anchor - 7 * 86_400))
    };

    if want_report {
        let cockpit = metrics::build_cockpit(&commits, &repos, &header);
        let churn = repo_churn(&cfg.repos, &labels, since)?;
        let tree = metrics::thrash_tree(&repos, &churn, since, recent_cut, multi);
        let rows = metrics::hotspots(&repo_files(&cfg.repos, &labels, since)?, 12);
        let heat = metrics::cadence_heatmap(&commits);
        let path = "tv-report.html";
        render::write_report(&cockpit, &tree, &rows, &heat, since.is_some(), multi, path)?;
        println!("wrote {path}");
        return Ok(());
    }

    match cfg.command {
        Command::Status => {
            let cockpit = metrics::build_cockpit(&commits, &repos, &header);
            render::print_cockpit(&cockpit, &palette);
        }
        Command::Thrash => {
            let churn = repo_churn(&cfg.repos, &labels, since)?;
            let tree = metrics::thrash_tree(&repos, &churn, since, recent_cut, multi);
            render::print_thrash(&header, &tree, since.is_some(), &palette);
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tv: {e}\n");
            print_help();
            exit(2);
        }
    };
    if let Err(e) = run(cfg) {
        eprintln!("tv: {e}");
        exit(1);
    }
}
