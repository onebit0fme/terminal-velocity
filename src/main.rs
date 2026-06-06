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

use std::process::exit;

enum Command {
    Status,
    Thrash,
    Hotspots,
    Explain,
}

struct Config {
    repo: String,
    command: Command,
    report: bool,
    color_off: bool,
    all_time: bool,
}

/// Default window for the drill-down commands — matches the cockpit's trailing
/// 8 weeks, anchored to the latest commit. `--all` widens to full history.
const WINDOW_SECS: i64 = 8 * 7 * 86_400;

fn window_since(repo: &str, all_time: bool) -> Result<Option<i64>, String> {
    if all_time {
        Ok(None)
    } else {
        Ok(Some(git::latest_ts(repo)? - WINDOW_SECS))
    }
}

fn parse_args() -> Result<Config, String> {
    let mut repo = ".".to_string();
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
            "--repo" => repo = it.next().ok_or("--repo requires a path")?,
            "status" => command = Command::Status,
            "thrash" => command = Command::Thrash,
            "hotspots" => command = Command::Hotspots,
            "explain" | "tree" => command = Command::Explain,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Config {
        repo,
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
    tv [COMMAND] [--repo <path>] [--report]

COMMANDS:
    status      one-screen build-flow cockpit (default)
    thrash      in-place rewrite (S-weighted) ranked by directory
    hotspots    files ranked by churn × complexity
    explain     print the heuristic decision tree

OPTIONS:
    --repo <path>   analyze this repo (default: current directory)
    --report        write a self-contained HTML report (tv-report.html)
    --no-color      disable color (also respects NO_COLOR)
    --all           thrash/hotspots over all history (default: last 8 weeks)
    -h, --help      show this help
    -V, --version   show version
"
    );
}

fn run(cfg: Config) -> Result<(), String> {
    let palette = style::Palette::detect(cfg.color_off);

    // Commands that need neither the commit list nor the blame pass.
    match cfg.command {
        Command::Explain => {
            render::print_explain(&palette);
            return Ok(());
        }
        Command::Hotspots => {
            let since = window_since(&cfg.repo, cfg.all_time)?;
            let freq = git::file_change_freq(&cfg.repo, since)?;
            let cx = git::file_complexity(&cfg.repo)?;
            let rows = metrics::hotspots(&freq, &cx, 12);
            render::print_hotspots(&rows, since.is_some(), &palette);
            return Ok(());
        }
        _ => {}
    }

    // Status + Thrash need the survival collection.
    let commits = git::load_commits(&cfg.repo)?;
    if commits.is_empty() {
        return Err("no non-merge commits found (is this a git repo with history?)".into());
    }
    let branch = git::current_branch(&cfg.repo).unwrap_or_else(|_| "HEAD".to_string());
    let collection = git::collect_cached(&cfg.repo, &commits)?;

    match cfg.command {
        Command::Status => {
            let cockpit = metrics::build_cockpit(&commits, &collection, &branch);
            if cfg.report {
                render::write_report(&cockpit, "tv-report.html")?;
                println!("wrote tv-report.html");
            } else {
                render::print_cockpit(&cockpit, &palette);
            }
        }
        Command::Thrash => {
            let anchor = commits.iter().map(|c| c.ts).max().unwrap_or(0);
            let since = if cfg.all_time {
                None
            } else {
                Some(anchor - WINDOW_SECS)
            };
            let recent_cut = if cfg.all_time {
                None
            } else {
                Some(anchor - 7 * 86_400)
            };
            let churn = git::file_churn_totals(&cfg.repo, since)?;
            let tree = metrics::thrash_tree(&collection, &churn, since, recent_cut);
            render::print_thrash(&branch, &tree, since.is_some(), &palette);
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
