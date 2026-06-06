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
BUILD FLOW: steady, batches creeping. No thrash spiral.
Watch: batch median 190→240, split smaller.
────────────────────────────────────────────────────────────
code survival  █▇▇▆▆▅▄▄▄▄▃▃▃▁▁▁  half-life ~491c / ~86d · 76% of lines still alive
  S(age) = a deleted line's odds of having lived this long;
  thrash and excision weight every death by it.
────────────────────────────────────────────────────────────
  flow      ▁▂▁▁▁█▃   ramping · ~19059 lines/wk
  batch     ▃▃▄▅▆▆▇   rising · median 190→240 (p78 for you)
            └ split smaller — cheapest flow win
  thrash    ▇▅█▃▃▁▆   low · 6.1% of churn
            └ low — your speed is real throughput, not thrashing
  excision  ▂█▂▂▁▆▃   healthy · 9.6% of churn
            └ deliberate scope-cutting (healthy)
  cadence   ▁▂▂▃▂▂▃   steady · nights 14% · weekends 9% (local, UTC-4)
────────────────────────────────────────────────────────────
net +334k (… added, … deleted) · run `tv thrash` / `tv hotspots` to drill in
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
   bounded-context, not by author. The one sanctioned exception is `--me`:
   **self-instrumentation** (you, from your own `git config`) — never a
   `--author=<someone-else>`, which would be exactly that forbidden tool.
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
| **cadence** | night/weekend share; drift = burnout tripwire | ✅ live · `tv cadence` punchcard |
| **net flow** | added − deleted; building vs consolidating | ✅ live |
| **intent mix** | feature/refactor/fix/… (heuristic) | ✅ live |
| **flow** | survival-weighted build-flow rate | ✅ live |
| **thrash** | in-place rewrite, S-weighted (the risk signal) | ✅ live · `tv thrash` by area |
| **excision** | wholesale removal (healthy scope-cutting) | ✅ live |
| **survival** | the S(age) curve + half-life + % still alive (Kaplan-Meier) | ✅ live · shown under the verdict |
| **hotspots** | churn × complexity, by file | ✅ live · `tv hotspots` |

`tv status` and `tv report` lead with the **survival curve** — S(age), a deleted
line's odds of having lived this long — because thrash and excision weight every
death by it. It's fit per repo (so each repo is judged by its own line lifetimes)
and shown as the curve shape, the half-life, and the still-alive fraction.

`tv thrash` ranks in-place rewrite by directory; `tv hotspots` ranks files by
churn × complexity; `tv cadence` draws a weekday × hour commit punchcard (when
work lands, in local time) — all aggregate by area/time, never by author.

`--me` (any command) scopes everything to your own commits, inferred from
`git config`. Two lenses: your *rework* (thrash/excision on commits where you
did the deleting) and your *code's durability* (the survival curve becomes how
long the lines you write last — introducer = you). It's self-only by design.

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
./target/release/tv thrash     # or: hotspots, cadence, explain
./target/release/tv status --me  # only my commits: my rework + how long my code lasts
./target/release/tv status --at 2026-03-01  # rewind: build flow as of a past date/commit
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

**Archaeology & period comparison.** `--at <point>` rewinds the as-of point from
HEAD to a past commit or date: survival `S(age)` is recomputed against *that*
tree, the trailing window ends at *that* moment (not now), and the blame cache is
keyed separately so archaeology never clobbers your everyday HEAD cache. A single
repo takes a rev or a date directly. Across several repos the anchor is **one
shared moment in time** — name one repo's commit and the rest snap to its
timestamp (a bare sha is rejected: it exists in only one of them). Compare two
periods by running `--at` twice and reading the cockpits side by side.

```sh
./target/release/tv status --at HEAD~200                       # as of 200 commits back
./target/release/tv status --at v1.2.0                         # as of a tag
./target/release/tv status --repo . --at "3 weeks ago"         # as of a relative date
./target/release/tv thrash --repo ../a --repo ../b --at a@v1.0 # pin repo a; b snaps to a's time
```

## Architecture

```
src/
  main.rs       CLI entry, arg parsing, --at anchor resolution, dispatch
  git.rs        git plumbing: log --numstat, blame-at-death, HEAD-keyed cache
  model.rs      domain + presentation types (Commit, Card, Cockpit, Intent)
  intent.rs     heuristic intent classifier (keyword + diff-shape; the API seam)
  survival.rs   Kaplan-Meier survival + half-life (the self-calibrating yardstick)
  spark.rs      sparklines, percentile-against-own-history, median
  metrics.rs    build the cockpit + thrash tree + hotspots (all live)
  verdict.rs    deterministic verdict composer (rule-based, offline)
  style.rs      zero-dep ANSI palette (NO_COLOR / --no-color aware)
  render.rs     terminal cockpit (default) + HTML report (--report)
```

## What's next

The core is live — survival, thrash/excision, hotspots, cadence, the cockpit, the
HTML report, multi-repo aggregation, `--me`, and `--at` archaeology all ship
today. Still open:

1. **True incremental cache** (`git.rs`) — a new commit currently triggers a full
   blame-at-death pass (tens of seconds on a large repo). Blame only the new
   commits and their changed-file survivors, so post-commit runs stay
   git-status-fast.
2. **Semantic line-matching** — separate true deaths from migrations/rewrites
   (the line-survival research's rigor bar) for a cleaner thrash/excision split.
3. **Intent classifier seam** — swap the keyword heuristic in `intent.rs` for a
   learned classifier via an external call, to sharpen the prose messages.
   Optional by design: `tv` always runs fully without it.

## Lineage

Grew out of a Python spike that validated the core: line-lifetime **survival
analysis** (no magic N-day window — the repo's own distribution is the axis),
the **thrash vs excision** split, and a falsification test showing thrash
predicts future fix activity beyond raw churn. Field-scanned against DORA / SPACE
/ DX Core 4, LinearB, CodeScene, Swarmia, and the line-survival research before
committing to this shape.

## What these methods are based on

`tv` is an engineering tool, not a research project, but every signal it computes
has a basis in the literature. What each idea draws on:

**Line-survival as the yardstick.** Lifetimes are estimated with the Kaplan–Meier
product-limit estimator, right-censored at the anchor commit — Kaplan & Meier,
*Nonparametric Estimation from Incomplete Observations*, JASA 1958
([doi](https://doi.org/10.1080/01621459.1958.10501452)). Applying survival
analysis at line granularity — and the finding that **repository identity
dominates** line survival (a gamma-frailty effect outweighing every structural
covariate), which is exactly why `tv` fits `S(age)` *per repo* rather than
pooling — is Gurov, *Code Lifespan Survival Analysis (CLSA)*, arXiv:2606.04993,
2026 ([abs](https://arxiv.org/abs/2606.04993)). That code ages and decays in the
first place is the long-standing result of Eick, Graves, Karr, Marron & Mockus,
*Does Code Decay?*, IEEE TSE 2001 ([doi](https://doi.org/10.1109/32.895984)).

**Thrash as a risk signal.** Reporting rewrite as a *share* of churn (not absolute
volume) follows Nagappan & Ball, *Use of Relative Code Churn Measures to Predict
System Defect Density*, ICSE 2005
([doi](https://doi.org/10.1145/1062455.1062514)) — relative churn predicts defect
density where absolute counts don't.

**Hotspots = complexity × change.** Ranking files by change-frequency ×
complexity, and the broader "behavioral code analysis" framing, is Adam Tornhill's
*Your Code as a Crime Scene* (Pragmatic Bookshelf, 2015) and *Software Design
X-Rays* (2018), built into [CodeScene](https://codescene.com). The cheap
complexity proxy — indentation depth — is Hindle, Godfrey & Holt, *Reading Beside
the Lines: Indentation as a Proxy for Complexity Metrics*, ICPC 2008
([doi](https://doi.org/10.1109/ICPC.2008.13)).

**The "vital few" drill-in cut.** Showing the folders/files that carry ~80% of the
heat (Pareto / cumulative-share) instead of an arbitrary top-N rests on the
defect-clustering result — ~80% of defects come from ~20% of modules — Boehm &
Basili, *Software Defect Reduction Top 10 List*, IEEE Computer 2001
([doi](https://doi.org/10.1109/2.962984)); see also Ostrand, Weyuker & Bell,
*Predicting the Location and Number of Faults in Large Software Systems*, IEEE TSE
2005 ([doi](https://doi.org/10.1109/TSE.2005.49)).

**Batch size and flow.** Treating lines/commit as batch size, and "smaller batches
flow faster," is Reinertsen, *The Principles of Product Development Flow*
(Celeritas, 2009), echoed by DORA's
[working-in-small-batches](https://dora.dev/capabilities/working-in-small-batches/)
capability.

**What `tv` deliberately is *not*.** Delivery outcomes — deploys, lead time, change
failure rate — belong to DORA, not git history: Forsgren, Humble & Kim,
*Accelerate* (IT Revolution, 2018). That productivity is multidimensional and must
not be collapsed into a single per-developer number is the explicit warning of
Forsgren, Storey, Maddila, Zimmermann, Houck & Butler, *The SPACE of Developer
Productivity*, ACM Queue 2021 ([doi](https://doi.org/10.1145/3454122.3454124)),
reinforced by Beck & Orosz's
[response to McKinsey](https://newsletter.pragmaticengineer.com/p/measuring-developer-productivity-part-2)
(2023) — together the basis for the **aggregate-by-subsystem-never-by-person**
rule. For current DevEx measurement framing, see DX's
[DX Core 4](https://getdx.com/news/introducing-the-dx-core-4/) (2024).

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
