# terminal velocity (`tv`)

**Is your build's speed real throughput, or just thrashing?**

A `git status` for build-flow health. Run `tv`, get one terminal screen: a plain
verdict, then 3–4 leading indicators — each a headline + sparkline + where-you-sit
vs. *your own* history + one action (or an explicit "ignore"). Mostly quiet;
surfaces the one or two things drifting.

It measures the **interior** of velocity — how code is produced and evolves — not
delivery outcomes (deploys, incidents, lead time live in your CI/CD dashboards).
A tachometer, not a speedometer. The name is also the pun: it runs in your
*terminal*, and *terminal velocity* is the honest, sustainable pace you actually
hold — not the burst you imagine.

```
terminal velocity · main · last 7d vs trailing 8wk
────────────────────────────────────────────────────────────
BUILD FLOW: steady, batches creeping. Watch: batch median
190→240, split smaller.
────────────────────────────────────────────────────────────
  flow      ·······  pending
            └ survival-weighted build flow — needs the blame pass
  batch     ▃▃▄▅▆▆▇   rising · median 190→240 (p78 for you)
            └ split smaller — cheapest flow win
  thrash    ·······  pending
  excision  ·······  pending
  cadence   ▁▂▂▃▂▂▃   steady · nights 14% · weekends 9% (local, UTC-4)
────────────────────────────────────────────────────────────
half-life ~(pending) · net +334k (… added, … deleted) · run
`tv thrash` / `tv hotspots` to drill in
```

## Why it exists

The job is **legibility**. Under shipping pressure you're usually building fine
but can't *see* the shape of it, so anxiety fills the vacuum. `tv` makes the shape
visible — and the reassurance line ("nothing drifting, you're building, not
spinning") matters as much as the alarms.

## Design non-negotiables

1. **Verdict before chart.** A human-readable "build flow is X; watch Y; Z is
   fine" on top. Charts are the drill-down. (Stops it being metric-theater.)
2. **Trends, not snapshots.** Every number is a sparkline against a trailing
   baseline. A *rising* batch median is the signal; the absolute 240 isn't.
3. **Your own distribution is the axis.** "p78 for you," never "elite vs low."
   No external benchmarks. (Line survival is dominated by repo identity, so a
   universal yardstick is statistically wrong anyway.)
4. **Aggregate by subsystem, never by person.** The load-bearing safety rule.
   Per-developer columns turn this into the surveillance tool the whole field's
   anti-metrics consensus forbids. `thrash`/`hotspots` group by directory /
   bounded-context, not by author.
5. **Each metric carries its action — or its "ignore."** "Batch rising → split."
   "Thrash spike → mechanical rename, ignore" vs "stabilize this area." The
   permission to ignore mechanical churn is as important as the alarm.
6. **Git-status-fast.** Daily glance < ~2s via an incremental, HEAD-keyed cache:
   only new commits get the expensive blame pass.
7. **Coverage honesty.** Too little history? Say "building baseline" — don't
   render a confident-but-fake percentile.
8. **Deterministic & offline.** The verdict is rule-composed from metric states —
   no network, no LLM at runtime. A learned classifier may *sharpen* intent
   labels later via an external call, but it's never required to run `tv`.

## Metrics

| metric | what | status |
|--------|------|--------|
| **batch** | lines/commit, weekly-median trend; small batches flow faster | ✅ live |
| **cadence** | night/weekend share; drift = burnout tripwire | ✅ live |
| **net flow** | added − deleted; building vs consolidating | ✅ live |
| **intent mix** | feature/refactor/fix/… (heuristic) | ✅ live |
| **flow** | survival-weighted build-flow rate | ✅ live |
| **thrash** | in-place rewrite, S-weighted (the risk signal) | ✅ live · `tv thrash` by area |
| **excision** | wholesale removal (healthy scope-cutting) | ✅ live |
| **half-life** | code-survival median (Kaplan-Meier) | ✅ live |
| **hotspots** | churn × complexity, by file | ✅ live · `tv hotspots` |

`tv thrash` ranks in-place rewrite by directory; `tv hotspots` ranks files by
churn × complexity — both aggregate by area, never by author.

## How it decides

Run **`tv explain`** to print the full heuristic decision tree right in the
terminal — the intent classifier, every state threshold, the verdict logic, and
which signals are self-calibrated vs. tunable constants. The fastest way to build
an accurate mental model of what `tv` is (and isn't) inferring.

## Install & run

Needs a Rust toolchain (`rustup`, then `cargo`):

```sh
cargo build --release          # zero dependencies; builds offline
./target/release/tv            # cockpit for the current repo
./target/release/tv --repo /path/to/repo
./target/release/tv thrash     # or: hotspots, explain
./target/release/tv report     # all three -> one HTML page (tv-report.html)
cargo test                     # survival-curve unit tests
```

`report` writes a self-contained HTML page with the cockpit, the thrash tree,
and the hotspots list. `--report` on any command is shorthand for the same page.

Repeat `--repo` to aggregate several repos. Survival `S(age)` is fit **per
repo** (repo frailty differs — a pooled curve would misweight every repo), and
`thrash` / `hotspots` attribute each row to its repo (folder tree rooted at the
repo; a `· repo` tag on each hotspot). The window is shared, anchored to the
newest commit across them.

```sh
./target/release/tv thrash --repo ../a --repo ../b
```

For a directory of projects — especially git-worktree layouts
(`project/<branch>/…`) — `scripts/scan-repos.sh` discovers one repo per project
(each project's `main`/`master` worktree, deduped, feature worktrees and nested
subtrees skipped) and aggregates them:

```sh
scripts/scan-repos.sh ~/code            # aggregated status
scripts/scan-repos.sh ~/code thrash     # any tv subcommand/flags
TV_DRY=1 scripts/scan-repos.sh ~/code   # just list the selection
mise run scan -- ~/code thrash          # same, builds release first
```

## Architecture

```
src/
  main.rs       CLI entry, arg parsing, dispatch
  git.rs        git plumbing (log --numstat now; blame-at-death + cache: TODO)
  model.rs      domain + presentation types (Commit, Card, Cockpit, Intent)
  intent.rs     heuristic intent classifier (keyword + diff-shape)
  survival.rs   Kaplan-Meier survival + half-life (the self-calibrating yardstick)
  spark.rs      sparklines, percentile-against-own-history, median
  metrics.rs    build the cockpit (batch/cadence/net live; flow/thrash pending)
  verdict.rs    deterministic verdict composer (rule-based, offline)
  render.rs     terminal cockpit (default) + HTML report (--report)
```

## Roadmap

1. **Blame-at-death + incremental cache** (`git.rs`) — ages each deleted line by
   blaming its range in the parent (`-w`); file add/delete balance splits
   deletions into **thrash** (rewrite, censored for survival) vs **excision**
   (true death, the KM event). Cache death-records keyed by HEAD sha; only blame
   new commits. Wires up flow / thrash / excision / half-life.
2. **`tv hotspots`** — churn × complexity by area (the highest-value drill-down).
3. **HTML report** — the 6-panel time-series view, topped with the verdict.
4. **Semantic line-matching** — separate true deaths from migrations/rewrites
   (the line-survival research's rigor bar) for cleaner thrash.
5. **Intent classifier API seam** — swap the heuristic for a learned classifier
   to sharpen the prose-message cases (optional, never required).

## Lineage

Grew out of a Python spike that validated the core: line-lifetime **survival
analysis** (no magic N-day window — the repo's own distribution is the axis),
the **thrash vs excision** split, and a falsification test showing thrash
predicts future fix activity beyond raw churn. Field-scanned against DORA / SPACE
/ DX Core 4, LinearB, CodeScene, Swarmia, and the line-survival research before
committing to this shape.

## License

MIT OR Apache-2.0 (license files TBD).
