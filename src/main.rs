//! terminal velocity (`tv`) — a git-status for build-flow health.
//!
//! Cockpit-first: a dense status board, git-status-shaped — 3-4 leading
//! indicators, each as a gutter status glyph + headline + sparkline +
//! where-you-sit-vs-your-own-history + an action or an explicit "ignore". No
//! composed verdict; the reader triages the glyph column.
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
    me: bool,
    at: Option<String>,
}

/// A repo's resolved as-of point: the git rev to drive every pass off
/// (`"HEAD"` by default, or the `--at` commit sha), and whether that is HEAD.
struct Anchor {
    rev: String,
    is_head: bool,
}

/// The filtered (repos, labels, per-repo anchors) plus an optional header note —
/// what `resolve_anchors` hands back.
type Resolved = (Vec<String>, Vec<String>, Vec<Anchor>, Option<String>);

/// Default window for the drill-down commands — matches the cockpit's trailing
/// 8 weeks, anchored to the latest commit. `--all` widens to full history.
const WINDOW_SECS: i64 = 8 * 7 * 86_400;

/// Newest anchored commit across all repos — the shared window anchor. With `--at`
/// this is the as-of moment, not now, so the window trails the anchor.
fn latest_ts_across(repos: &[String], anchors: &[Anchor]) -> Result<i64, String> {
    let mut latest: Option<i64> = None;
    for (r, a) in repos.iter().zip(anchors) {
        let ts = git::commit_ts(r, &a.rev).map_err(|e| format!("{r}: {e}"))?;
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

/// Does this string read as a date rather than a revision? Used to decide whether
/// a non-rev `--at` value should be handed to git's date parser. Keeps a typo'd
/// sha from silently resolving to "now" — it must look date-ish to take that path.
fn looks_like_date(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    const KW: &[&str] = &[
        "ago",
        "yesterday",
        "today",
        "now",
        "week",
        "month",
        "year",
        "day",
        "hour",
        "minute",
        "noon",
        "midnight",
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
    ];
    if KW.iter().any(|k| l.contains(k)) {
        return true;
    }
    let has_digit = l.bytes().any(|b| b.is_ascii_digit());
    let has_sep = l.contains('-') || l.contains('/') || l.contains(':');
    has_digit && has_sep
}

/// Resolve a single-repo `--at` value to a commit sha: a git revision if it is one,
/// otherwise a date snapped to the last commit on/before it. Fail-loud on junk.
fn resolve_rev_or_date(repo: &str, s: &str) -> Result<String, String> {
    if let Ok(sha) = git::resolve_rev(repo, s) {
        return Ok(sha);
    }
    if looks_like_date(s) {
        return match git::snap_before(repo, s)? {
            Some(sha) => Ok(sha),
            None => Err(format!("no commit on/before `{s}`")),
        };
    }
    Err(format!(
        "`{s}` isn't a revision here, and doesn't look like a date"
    ))
}

/// Resolve the per-repo `--at` anchor. Returns the (possibly filtered) repos/labels
/// alongside one [`Anchor`] each, plus a human note for the header.
///
/// Multi-repo collapses to a single shared moment in time:
///   - `label@rev` — that repo pins to the exact commit; its timestamp snaps the rest.
///   - a date — every repo snaps to its last commit on/before that date.
///
/// A bare revision is rejected across repos (a sha exists in only one of them).
fn resolve_anchors(
    paths: &[String],
    labels: &[String],
    at: Option<&str>,
) -> Result<Resolved, String> {
    let Some(spec) = at else {
        let anchors = paths
            .iter()
            .map(|_| Anchor {
                rev: "HEAD".to_string(),
                is_head: true,
            })
            .collect();
        return Ok((paths.to_vec(), labels.to_vec(), anchors, None));
    };
    let multi = paths.len() > 1;

    // Reduce the spec to: an optional exact pin (one repo, one sha) + a `--before`
    // expression that snaps every other repo to the same moment.
    let mut pin: Option<(usize, String)> = None;
    let mut before: Option<String> = None;
    let mut note: Option<String> = None;

    if let Some((pre, rev)) = spec.split_once('@') {
        if let Some(idx) = labels.iter().position(|l| l == pre) {
            let sha = resolve_rev_or_date(&paths[idx], rev)?;
            let ts = git::commit_ts(&paths[idx], &sha)?;
            let date = git::commit_date(&paths[idx], &sha).unwrap_or_default();
            note = Some(format!("@{pre} {} ({date})", &sha[..sha.len().min(8)]));
            pin = Some((idx, sha));
            before = Some(format!("@{ts}"));
        }
    }

    if pin.is_none() && before.is_none() {
        if multi {
            if !looks_like_date(spec) {
                let example = labels.first().map(String::as_str).unwrap_or("repo");
                return Err(format!(
                    "with several repos, anchor to one repo's commit (e.g. `{example}@{spec}`) \
                     or give a date — a bare revision is ambiguous across repos"
                ));
            }
            note = Some(format!("as of {spec}"));
            before = Some(spec.to_string());
        } else {
            let sha = resolve_rev_or_date(&paths[0], spec)?;
            let dated = git::resolve_rev(&paths[0], spec).is_err() && looks_like_date(spec);
            note = Some(if dated {
                format!("as of {spec} ({})", &sha[..sha.len().min(8)])
            } else {
                let date = git::commit_date(&paths[0], &sha).unwrap_or_default();
                format!("@{} ({date})", &sha[..sha.len().min(8)])
            });
            pin = Some((0, sha));
        }
    }

    // Materialize per-repo anchors, dropping repos with no history that old.
    let mut out_paths = Vec::new();
    let mut out_labels = Vec::new();
    let mut anchors = Vec::new();
    for (i, (p, l)) in paths.iter().zip(labels).enumerate() {
        let sha = match &pin {
            Some((pidx, psha)) if *pidx == i => Some(psha.clone()),
            _ => match &before {
                Some(b) => git::snap_before(p, b).map_err(|e| format!("{p}: {e}"))?,
                None => None,
            },
        };
        let Some(sha) = sha else {
            eprintln!("tv: {l}: no commit on/before the anchor — skipping");
            continue;
        };
        let is_head = git::head_sha(p).map(|h| h == sha).unwrap_or(false);
        anchors.push(Anchor { rev: sha, is_head });
        out_paths.push(p.clone());
        out_labels.push(l.clone());
    }
    if anchors.is_empty() {
        return Err("no repo has any commit on/before the anchor".to_string());
    }
    Ok((out_paths, out_labels, anchors, note))
}

fn parse_args() -> Result<Config, String> {
    let mut repos: Vec<String> = Vec::new();
    let mut command = Command::Status;
    let mut report = false;
    let mut color_off = false;
    let mut all_time = false;
    let mut me = false;
    let mut at: Option<String> = None;
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
            "--me" => me = true,
            "--at" => at = Some(it.next().ok_or("--at requires a commit, ref, or date")?),
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
        me,
        at,
    })
}

fn print_help() {
    print!(
        "\
terminal velocity (tv) — is your build's speed real throughput, or just thrashing?

USAGE:
    tv [COMMAND] [--repo <path>]... [--at <point>] [--me] [--report]

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
    --me            only my own commits (from git config); my rework + how long
                    the code I write survives. Self only — no per-teammate view.
    --at <point>    rewind the as-of point for archaeology / period comparison
                    (default: HEAD). A single repo takes a rev or date:
                      --at v1.2.0   --at HEAD~50   --at 2026-03-01   --at \"3 weeks ago\"
                    Across several repos the anchor is one shared moment in time:
                    name one repo's commit and the rest snap to that timestamp —
                      --at myrepo@a1b2c3d        (or just a date: --at 2026-03-01)
                    A bare revision is rejected for multi-repo (a sha lives in
                    one repo). Compare two periods by running --at twice.
    -h, --help      show this help
    -V, --version   show version
"
    );
}

/// Per-repo (label, change-frequency, complexity) — the hotspots inputs.
/// `anchors` cap each repo's history at its `--at` point; `authors` (non-empty
/// under `--me`) restricts the change counts to my commits.
fn repo_files(
    repos: &[String],
    labels: &[String],
    anchors: &[Anchor],
    since: Option<i64>,
    authors: &[String],
) -> Result<Vec<metrics::RepoFiles>, String> {
    let mut out = Vec::new();
    for ((repo, label), a) in repos.iter().zip(labels).zip(anchors) {
        let f = git::file_change_freq(repo, &a.rev, since, authors)
            .map_err(|e| format!("{repo}: {e}"))?;
        let c =
            git::file_complexity(repo, &a.rev, a.is_head).map_err(|e| format!("{repo}: {e}"))?;
        out.push((label.clone(), f, c));
    }
    Ok(out)
}

/// Per-repo (label, churn-by-path) — the denominator for the thrash %.
fn repo_churn(
    repos: &[String],
    labels: &[String],
    anchors: &[Anchor],
    since: Option<i64>,
    authors: &[String],
) -> Result<Vec<metrics::RepoChurn>, String> {
    let mut out = Vec::new();
    for ((repo, label), a) in repos.iter().zip(labels).zip(anchors) {
        let c = git::file_churn_totals(repo, &a.rev, since, authors)
            .map_err(|e| format!("{repo}: {e}"))?;
        out.push((label.clone(), c));
    }
    Ok(out)
}

fn run(cfg: Config) -> Result<(), String> {
    let palette = style::Palette::detect(cfg.color_off);

    // Explain is pure prose — no repos, no anchor resolution.
    if let Command::Explain = cfg.command {
        render::print_explain(&palette);
        return Ok(());
    }

    // Resolve the `--at` anchor first: it can rewind, and even drop, repos. Every
    // pass below keys off these per-repo anchors instead of an implicit HEAD.
    let base_labels = repo_labels(&cfg.repos);
    let (paths, labels, anchors, anchor_note) =
        resolve_anchors(&cfg.repos, &base_labels, cfg.at.as_deref())?;
    let multi = paths.len() > 1;
    // Date of the newest anchor (the window's upper edge) — `None` at HEAD. The
    // renderers use it to turn "last 7d" into an explicit "as of <date>" frame.
    let as_of: Option<String> = if cfg.at.is_some() {
        let mut best_ts = i64::MIN;
        let mut best_date: Option<String> = None;
        for (a, p) in anchors.iter().zip(&paths) {
            if let Ok(ts) = git::commit_ts(p, &a.rev) {
                if ts > best_ts {
                    best_ts = ts;
                    best_date = git::commit_date(p, &a.rev).ok();
                }
            }
        }
        best_date
    } else {
        None
    };
    // `report` command and the `--report` flag are the same thing: the full page.
    let want_report = cfg.report || matches!(cfg.command, Command::Report);

    // `--me`: resolve my identity from git config (self-instrumentation only).
    let me: Option<model::Me> = if cfg.me {
        Some(
            paths
                .iter()
                .find_map(|r| git::whoami(r))
                .ok_or("--me needs your identity — set `git config user.email`")?,
        )
    } else {
        None
    };
    // `--author` patterns for the log-based file metrics (git OR-matches email/name).
    let author_pats: Vec<String> = me
        .as_ref()
        .map(|m| vec![m.email.clone(), m.name.clone()])
        .unwrap_or_default();
    let header = {
        let mut s = scope_label(&paths);
        if let Some(n) = &anchor_note {
            s = format!("{s} · {n}");
        }
        if me.is_some() {
            s = format!("{s} · me");
        }
        s
    };
    let keep_mine = |commits: &mut Vec<model::Commit>| {
        if let Some(m) = &me {
            commits.retain(|c| c.by(m));
        }
    };
    let empty_err = || -> String {
        match &me {
            Some(m) => format!("no commits by you ({}) found in the given repo(s)", m.email),
            None => "no non-merge commits found across the given repo(s)".into(),
        }
    };

    match cfg.command {
        // Hotspots to the terminal needs no blame pass; the report does, so it
        // falls through to the full pipeline below.
        Command::Hotspots if !want_report => {
            let since = if cfg.all_time {
                None
            } else {
                Some(latest_ts_across(&paths, &anchors)? - WINDOW_SECS)
            };
            let per_repo = repo_files(&paths, &labels, &anchors, since, &author_pats)?;
            let rows = metrics::hotspots(&per_repo);
            render::print_hotspots(
                &header,
                &rows,
                since.is_some(),
                multi,
                as_of.as_deref(),
                &palette,
            );
            return Ok(());
        }
        // Cadence is a commit-time punchcard — commits only, no blame pass.
        Command::Cadence if !want_report => {
            let mut commits: Vec<model::Commit> = Vec::new();
            for (repo, a) in paths.iter().zip(&anchors) {
                commits
                    .extend(git::load_commits(repo, &a.rev).map_err(|e| format!("{repo}: {e}"))?);
            }
            keep_mine(&mut commits);
            if commits.is_empty() {
                return Err(empty_err());
            }
            let heat = metrics::cadence_heatmap(&commits);
            render::print_heatmap(&heat, &header, &palette);
            return Ok(());
        }
        _ => {}
    }

    // Status / Thrash / Report all need the per-repo survival collections (S is
    // fit per-repo — repo frailty differs — and output is attributed by repo).
    let mut commits: Vec<model::Commit> = Vec::new();
    let mut cols: Vec<(String, git::Collection)> = Vec::new();
    for ((repo, label), a) in paths.iter().zip(&labels).zip(&anchors) {
        let cs = git::load_commits(repo, &a.rev).map_err(|e| format!("{repo}: {e}"))?;
        if cs.is_empty() {
            continue; // an empty repo in a multi-repo set just contributes nothing
        }
        let col = git::collect_cached(repo, &cs, &a.rev).map_err(|e| format!("{repo}: {e}"))?;
        cols.push((label.clone(), col));
        commits.extend(cs);
    }
    keep_mine(&mut commits); // activity metrics + churn denominator are my commits
    if commits.is_empty() {
        return Err(empty_err());
    }

    // The window trails the anchor moment (the newest loaded commit), not now.
    let anchor_ts = commits.iter().map(|c| c.ts).max().unwrap_or(0);
    let (since, recent_cut) = if cfg.all_time {
        (None, None)
    } else {
        (Some(anchor_ts - WINDOW_SECS), Some(anchor_ts - 7 * 86_400))
    };

    if want_report {
        let cockpit =
            metrics::build_cockpit(&commits, &cols, &header, me.as_ref(), as_of.as_deref());
        let churn = repo_churn(&paths, &labels, &anchors, since, &author_pats)?;
        let tree = metrics::thrash_tree(&cols, &churn, since, recent_cut, multi, me.as_ref());
        let files = repo_files(&paths, &labels, &anchors, since, &author_pats)?;
        let rows = metrics::hotspots(&files);
        let heat = metrics::cadence_heatmap(&commits);
        let path = "tv-report.html";
        render::write_report(&cockpit, &tree, &rows, &heat, since.is_some(), multi, path)?;
        println!("wrote {path}");
        return Ok(());
    }

    match cfg.command {
        Command::Status => {
            let cockpit =
                metrics::build_cockpit(&commits, &cols, &header, me.as_ref(), as_of.as_deref());
            render::print_cockpit(&cockpit, &palette);
        }
        Command::Thrash => {
            let churn = repo_churn(&paths, &labels, &anchors, since, &author_pats)?;
            let tree = metrics::thrash_tree(&cols, &churn, since, recent_cut, multi, me.as_ref());
            render::print_thrash(&header, &tree, since.is_some(), as_of.as_deref(), &palette);
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
