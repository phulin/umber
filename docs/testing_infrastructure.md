# Testing Infrastructure

Status: current repository reference
Scope: the test commands, measured budgets, fixtures, corpora, and harnesses
that exist in this workspace today.

This document records current implementation facts: what each tool is and how
to run it. For rules that should guide future test design and placement, see
[Rust Testing Policy](testing_policy.md). For the _process_ of working a
`umber2-johp` canonical/oracle divergence with these tools -- diagnosis order,
oracle hierarchy, fix discipline, gates, and the glossary defining that
vocabulary -- see [Canonical Divergence Working Contract](canonical_divergence_workflow.md).
This document does not restate that process.

---

## Local Gates And Budgets

The fixture-only, hermetic correctness tier includes the TeX82 property-catalogue gate in `test-support`. It validates the committed 1,380-module inventory, generated default deferrals, deterministically merged domain-local disposition/property shards, canonical citations, ownership, and exact Rust test links without reading or invoking a live reference engine. See [TeX82 Property Catalogue](tex82_property_catalogue.md).

The fixture-only, hermetic correctness tier is:

```bash
cargo test --tests
scripts/check-and-test.sh
```

These commands run every workspace member whose tests can execute on the host;
see [What The Native Test Gate Covers](#what-the-native-test-gate-covers) for
how that set is established. What they do _not_ run -- the browser adapter, the
opt-in host tools, and the C HarfBuzz cross-check -- is covered by three named
tiers described in [Deferred Test Tiers](#deferred-test-tiers), and both
commands print what each of those tiers last did.
Generated-input stabilization uses the hermetic shared fixtures under
`tests/corpus/stabilization`: native unit tests consume them directly, while
the wasm-bindgen browser suite runs the same bytes and compares binary output,
generated files, pass counts, and typed fixed-point failures with the native
surface.
The WASM target reserves a 4 MiB linear-memory stack because retained compile
sessions exceed wasm-ld's 1 MiB default during Firefox retry and incremental
HTML coverage; native targets keep their platform stack policy.

A warmed `cargo test --tests` spends about 19 seconds inside its test
binaries, summed from the 48 `test result:` lines it prints. That figure is
worth stating because it is the one part of the run that does not depend on
build state: it is the same whether the tree is cold, warm, or contended.
Investigate any default test that invokes live TeX.

Wall-clock budgets are deliberately not stated here. The previous one --
"under 10 seconds on the current macOS development workspace; investigate a
sustained run above 15 seconds" -- was a number from one machine that no run
on other hardware could satisfy, and a threshold every run exceeds is one
nobody acts on. Replacing it with a second machine's number reproduces the
defect, and the replacement attempt demonstrated why: on the Linux workspace
the same warm command measured 17.9s, 22.4s, 25.2s, 33.0s, and 41.9s in one
sitting, drifting upward as an unrelated `k3s` workload took the load average
to 40 on 24 cores with swap fully exhausted.

So: measure wall clock against itself, back to back and alternating, on a
quiet machine, and never compare a figure taken before a `Cargo.toml` edit
with one taken after -- a feature change invalidates the build, and the
rebuild lands in whichever run happens to follow it. Two of this repository's
own optimization proposals died on exactly that mistake.

`scripts/check.sh` checks dprint and
rustfmt formatting, then runs clippy without rerunning tests; it has a warmed
two-minute local budget. Naming gates on its command line, as in
`scripts/check.sh clippy`, runs exactly those gates with the same commands the
full run uses; `scripts/check.sh node-width-budget` is the explicit form of the
`CHECK_BENCH=1` opt-in. That argument form exists so no one retypes a gate's
invocation by hand: a bare `cargo clippy` exits 0 on warn-level lints, and
`cargo clippy -p <crate>` resolves a different feature union than the gate, so
a hand-written clippy run can be green while the gate is red. Only
`scripts/check.sh` output may be reported as a gate result.
`scripts/check-and-test.sh` runs the native correctness suite concurrently with
that quality gate.

### What The Native Test Gate Covers

`cargo test --tests` selects the workspace's _default_ members. Every other
member -- the nine `bib-*` crates, `umber-wasm`, `umber-interrupt`, `refexec`,
and `profile-analyzer` -- was therefore executed by no routine command, and
`bib-engine`'s integration binary alone holds 1295 tests that nothing ran
(`umber2-johp.211`). Nothing had rotted when the gap was measured: all thirteen
members compiled and passed. That is the danger rather than the reassurance,
because `tools/tex-command-stream` had rotted out of compiling under exactly
the same gap (`umber2-johp.121`) and nobody found out from a test run.

The gap was closed by correcting `default-members` to name every host-testable
member, and by making that correctness a test rather than a wrapper script.
`crates/test-support/tests/workspace_selection.rs` holds two:

- `default_members_cover_every_host_testable_crate` reads `cargo metadata` and
  fails if any member is absent from `default-members` without an `OMITTED`
  entry naming the tier that runs it. A stale entry -- one naming a package the
  workspace no longer has, or one contradicted by `default-members` selecting
  it anyway -- fails too, so an excuse cannot outlive the thing it excused;
- `every_excluded_workspace_directory_names_its_tier` does the same for
  `[workspace] exclude` directories, which `--workspace` cannot reach at all
  because each is its own workspace with its own lockfile. Pushing a crate out
  of the workspace cannot quietly take its tests out of every gate on the way.

`scripts/run-native-tests.py` used to enforce this by wrapping Cargo. Enforcing
it from inside the suite is strictly better: the invariant now holds under the
command everyone already runs, rather than under one they have to remember,
and roughly 970 lines of wrapper, self-test, and asset-bootstrap script were
deleted with it.

`umber-wasm` is the single declared omission. Its tests are
`#[wasm_bindgen_test]`, which registers no test on a host target: selecting it
builds a cdylib and reports three test binaries containing zero tests, for
about six seconds of incremental link time and roughly 160 seconds on a cold
tree. `scripts/check-wasm.sh` runs those tests for real with
`wasm-pack test --headless --firefox crates/umber-wasm`.

The three `[workspace] exclude` directories -- `tools/corpus-sync`,
`tools/fixturegen`, `tools/texlive-wasm-publish` -- hold 23 tests between them
that likewise ran nowhere. They are separate workspaces with separate lockfiles
and target directories, so they belong in the explicit tool tier rather than in
the routine one: `scripts/check-tools.sh` runs each by manifest path, and
`every_excluded_workspace_directory_names_its_tier` fails if a fourth appears
without naming its gate.

Each exclusion names a tier rather than a free-text command, and the tier must
be one `scripts/tier_stamp.py` registers. That is what makes the deferral
checkable: a registered tier stamps its runs and appears in the report below,
so deferring to an unregistered name -- or to a typo -- fails with `COVERAGE`
instead of reading as coverage.

### Deferred Test Tiers

Six tiers are deliberately outside the routine gate, because running them
there would make the fast path depend on wasm-pack, a headless browser,
ripgrep, HarfBuzz, and three dependency trees the workspace lockfile does not
cover:

| Tier                                 | Covers                                                                                                                            |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `scripts/check-tools.sh`             | the three `[workspace] exclude` directories, `parity-harness` in its `reference-tools` resolution, and the opt-in clippy features |
| `scripts/check-wasm.sh`              | `umber-wasm`'s `#[wasm_bindgen_test]` suite and the authored browser package                                                      |
| `scripts/check-hb-shape-fixtures.sh` | the rustybuzz cross-check against C HarfBuzz                                                                                      |
| `scripts/check-latex-corpus.sh`      | the pinned native LaTeX base-class corpus and runtime closure                                                                     |
| `scripts/check-latex-wasm.sh`        | the pinned LaTeX native/WASM article parity build                                                                                 |
| `scripts/check-latex-parity.sh`      | the pinned upstream LaTeX2e DVI parity cohort                                                                                     |

Being separate was never the defect. The defect (`umber2-johp.213`) was that
the routine gates asserted these tiers cover what they exclude while nothing
invoked any of them and nothing recorded whether any had ever run: a green
routine run read as a coverage statement it had no evidence for. Three things
now hold instead.

**Each tier reports what it ran.** They share `scripts/tier-runner.sh`, which
gives every tier named steps and ends the run in a verdict line:

```text
check-tools.sh: VERDICT: PASS - 12 of 12 steps ran
check-tools.sh: VERDICT: BLOCKED - 11 of 12 steps ran, 1 could not run: arxiv-entrypoint needs rg, not installed here
check-tools.sh: VERDICT: FAIL - 12 of 12 steps ran, 1 failed: oracle-regeneration
check-tools.sh: VERDICT: PARTIAL - 2 of 12 steps ran; 10 not selected
```

Every step runs even after one fails, as in `scripts/check.sh`. A step whose
tool is absent is `BLOCKED`, never skipped, and `BLOCKED` exits 4: the exit
contract is `0` PASS or PARTIAL, `1` FAIL, `2` a step name the tier does not
have, `4` BLOCKED. Naming steps on the command line runs exactly those, with
byte-identical commands, and is recorded as the partial run it is.

**Each run is recorded where the routine gates read it.** `tier_finish` writes
a stamp under `.tier-stamps/` (gitignored) holding the commit, whether the tree
was dirty, and the step census. `scripts/tier_stamp.py report` classifies each
tier against the tree in front of you as `PASSED`, `PARTIAL`, `STALE`,
`BLOCKED`, `FAILED`, or `NEVER-RUN`, and only a whole clean run at HEAD counts
as evidence. `scripts/check.sh` prints that report -- reading a file, running
no tier -- so a green suite can no longer be mistaken for a statement about the
deferred tiers:

```text
TIER SUMMARY: 0 of 6 deferred tiers have passed on this tree (6 never-run)
```

`scripts/test_tier_stamp.py` drives the classifier with every shape that must
_not_ count as evidence -- a stale commit, a dirty tree, a named-step subset, a
blocked run -- and `tier_stamp.py report` runs it before printing anything, for
the same reason the clippy gate self-tests `check-lint-passes.py`.
`scripts/test-deferred-tier-entrypoints.py`, which the native gate runs before
its own verdict, verifies that the stamped LaTeX entries are registered,
discoverable, and report a missing prerequisite as BLOCKED.

**Something invokes them.** `.github/workflows/deferred-tiers.yml` runs
`check-wasm.sh`, `check-tools.sh`, and `check-hb-shape-fixtures.sh` on every
pull request, on pushes to `main`, and weekly; the HarfBuzz job installs the
`hb-shape` provider explicitly. The LaTeX tiers remain manual because they
need the pinned live distribution and reference inputs, but their entry points
write the same stamps and are therefore visible in ordinary verdicts. Locally,
`scripts/hooks/pre-push` refuses to push a branch while any tier
has never been invoked in that checkout. It refuses only `NEVER-RUN`: a tier
that answered `BLOCKED` because a tool is absent has told you something, while
refusing `BLOCKED` and `FAILED` too would make recording a bad outcome worse
for the author than never running the tier at all. Install the hooks with
`scripts/install-hooks.sh`, which prints what it installed.

The repository format and lint contract is independent of those expensive
tiers. `.github/workflows/quality.yml` runs `scripts/check.sh` without a path
filter on every pull request and every push to `main`, so documentation, root
Markdown, fixtures, scripts, tools, and every workspace crate reach the same
gate. The deferred WASM job runs native correctness through
`cargo test --tests` but does not repeat formatting or linting.
`scripts/test-ci-workflows.py` binds those workflow responsibilities so a
future edit cannot silently put the path-filter hole back.

A tier can be BLOCKED on a normal development machine, and that is the point:
on a host without ripgrep, `check-tools.sh` reports
`VERDICT: BLOCKED - 11 of 12 steps ran, 1 could not run` and exits 4 rather
than reporting a pass for a comparison that never happened.

Roughly 940 of the tests the gate now runs report as ignored. They are
`bib-engine`'s `#[ignore = "xfail: <specific production gap>"]` markers against
the pinned upstream biber compatibility suite, audited by
`tests/it/scaffold.rs`; the verdict line states the count rather than hiding it
behind a total.

### Declarative Command Semantic Minifixtures

Run the fast property-scoped semantic tier independently with:

```bash
cargo test -q -p tex-command-stream --test it command_semantic
```

The one Cargo integration binary discovers independent domain manifests under
`tests/corpus/command-semantic/<domain>/`; adding a domain does not edit a shared
Rust registry or add a top-level integration target. Each versioned manifest
binds a tiny source to its catalogue property, exact canonical authority and
sections, projection kind, short expected observations, and either `pass` or a
strict `xfail` expectation. Discovery rejects malformed, duplicate, unsafe, or
unowned cases and sources. A manifest whose short directory name differs from
its catalogue shard declares `property_domain`; ownership validation remains
exact.

The corpus contract -- manifest parsing and validation, the bounded canonical
run, and the projections -- lives in `tex_command_stream::semantic`, not in the
test binary, so the regeneration path drives exactly the code the gate does.
The test binary holds only the assertions.

The runner drives each input through instrumented
`tex_exec::CanonicalMainControl` in the exact TeX82 INITEX profile. A run is a
real TeX job, not a bare command loop: it is framed with
`CanonicalMainControl::begin_job`/`finish_job` exactly the way
`docs/job_framing.md` describes -- the start-up banner, the `**` line, the
root source registered by name so §537/§362 bracket it in `(name`/`)`, and
§642's page report and transcript line once the run ends -- and it runs in
`InteractionMode::Scroll`, matching the oracle runner's own
`-interaction=scrollmode` (see that script's "Interaction mode" comment for
why: scrollmode is the one mode that both tolerates the `\read`/`\pausing`
cases this corpus feeds terminal answers to and omits the error-stop prompt
an undeclared divergence would otherwise demand an answer for). The two
profiles built past INITEX (`etex-loaded`, `production`) cannot take
`begin_job`'s INITEX-only banner, and the oracle runner cannot reproduce
either profile at all, so their 5 cases run unframed, exactly as every case
did before this framing existed. The runner compares the declared concise
projection of committed command observations or selected
canonical-main-control boundaries -- mode changes, final box-register node
outlines, and committed shipout artifact identities -- and, separately, the
per-channel contract described below. An xfail must link a concrete Beads bug
and pin the first mismatch's index, kind, expected value, and actual value;
XPASS and changed-failure results fail the test. Nothing uses `#[ignore]`,
`should_panic`, a live TeX process, a format or fonts, or the generated
long-document trace registry.

The corpus holds 130 cases across 8 domains, with 8 strict xfails: 3 in
`alignments`, 3 in `input-expansion`, and 2 in `main-control`. The other five
domains carry none. Bounded in-memory terminal lines and named inputs keep the
pausing, read, and input-open evidence hermetic.

#### The Minimality Contract

A minifixture is truly minimal: short, self-contained, loading no format and
no macro package, containing only what is needed to exercise the one engine
behavior its case is about. `validate_case` enforces this, so a violating
source fails the gate rather than merely reading as unusual in review:

- **No format or package loading.** A source may not reference `plain.tex` or
  `\input plain`, and may not `\input` a file its case does not declare in its
  `inputs` map (the same map that already backs `\openin`/read-stream cases).
  Two committed cases legitimately `\input` a companion file --
  `input-expansion/input-start-file` (`\input nested`) and
  `input-expansion/input-level-lifecycle` (`\input child.tex`) -- and pass
  because both targets are declared in those cases' `inputs` maps, not because
  of anything naming the case. `\dump` is deliberately not forbidden: it writes
  a format rather than loading one, so it does not bear on minimality, and
  `main-control/final-cleanup-end-or-dump` exists to exercise tex.web §1335's
  rejection of it. Forbidding it would have taken an exception carved to fit
  that one source, which is the shape of rule that stops meaning anything. The
  undeclared-`\input` check is what actually prevents a fixture assembling a
  format, and it applies to every case alike.
- **A byte ceiling**, `MAX_SOURCE_BYTES`. The observed maximum across the
  corpus is 1,240 bytes (`etex-diagnostics/etex-expressions.tex`), so the
  ceiling is 2,048 bytes: real headroom over every committed case, not the
  4,096 the corpus never came close to.
- **A line ceiling**, `MAX_SOURCE_LINES`. The observed maximum is 31 lines
  (`main-control/spacefactor-assignment.tex`), so the ceiling is 64 lines, on
  the same reasoning.

Both constants and the format-loading check live in `tex_command_stream::semantic`
alongside the rest of the corpus contract, with unit tests proving each rule's
accept and reject direction in `tex-command-stream/src/semantic/tests.rs`.

#### The Per-Channel Contract

A projection is a focused property claim about one observable. It is not
coverage of the run, and for a long time it was standing in for one. Measured
across the corpus, the 130 cases produce 33,112 events, 23,013 bytes of
terminal and log text, and 26 shipped DVI pages, against 698 declared
assertion strings -- a mean of 5.5 per case. 26 cases ship a page that nothing
compared; 39 write a log, and the log was read by no projection that exists,
because `terminal-checks` is a `contains()` boolean over the merged terminal
text. **A projection is an omission with a schema**, which is the same defect
as `default-members` naming 21 of 34 crates: an absence that reads as
coverage.

So each case declares a `channels` block accounting for every observable its
run produces, and the gate compares all of them alongside the projection:

- `events`, the exact committed-observation count. Counted rather than
  committed, because the canonical event stream's oracle-backed home is the
  `tests/corpus/command/tex82` fixture tree and duplicating it here would
  commit an Umber self-golden;
- `status`, either `clean` or `fatal:<label>` for a §81 `jump_out`;
- `terminal`, `log`, `dvi`, and `effects`, each `empty`, `file`, or `xfail`.
  A committed `expected/<case-id>.<channel>` file is required for the latter
  two. The corpus commits 274 such files today: 124 terminal, 124 log, and 26
  dvi. Terminal and log both grew from a minority of cases to nearly every one
  once job framing gave every run a banner, a `**` line, and a page report or
  "No pages of output." to write, where previously only a case with its own
  diagnostic output produced either channel at all.

The `dvi` channel is the run's complete serialized `.dvi` file, built with
`tex_out::dvi::DviStreamWriter` over the same `DviPagePlan`s
`umber::dvi_from_page_plans` assembles, not a description of one. It used to
be a `page:<index>:<content-hash>` line per shipped page: a hash listing that
could never be checked against the oracle's own `.dvi` file, since there is
no oracle hash to compare it to -- only oracle bytes. Byte-exact comparison
against a pinned reference engine is the whole point of this corpus
(`umber2-alfh.1`), so the `dvi` channel had to become the same _kind_ of
object the oracle's `.dvi` file is before that comparison could exist at all.
Because those bytes are binary rather than line-oriented text,
`CapturedChannels`' four stream channels are `Vec<u8>` rather than `String`,
and the channel-content comparison decides byte equality on the raw bytes
first, falling back to a lossy UTF-8 rendering only to describe a divergent
line in a failure report -- so a real divergence in binary content can never
be masked by a lossy decode the way comparing pre-decoded `String`s would
risk.

A case with no `channels` block fails validation. The one exemption is a case
whose engine run does not complete and therefore has no channels to record;
it is granted only to a case already pinned as `xfail`, so it expires with the
bug instead of becoming the escape hatch. Three cases hold it --
`input-expansion/expansion-conversions`, `input-expansion/input-start-file`,
and `main-control/read-to-definition` -- and
`only_unrunnable_xfail_cases_are_exempt_from_the_channel_contract` pins that
set by name and re-runs each to prove it still cannot run.

Every committed channel records an `authority` for its bytes. All 274 are
`umber-baseline` today: they pin the channel against silent drift and are still
owed an oracle adjudication, which `authority: "oracle"` records once a
reference engine has produced them. The count is greppable, so the remaining
work is visible rather than assumed. Promoting them is `umber2-alfh.1`, which
job framing unblocks but does not itself perform: comparing the regenerated
`terminal`/`log` channels against a pinned pdfTeX 1.40.27 oracle capture
(`scripts/run-minifixture-oracle.sh`, after only the documented clock
normalization) now finds 29 of 122 comparable log channels and 0 of 122
comparable terminal channels byte-identical, up from 0 of 39 log channels
before this framing existed. Of the remainder, 30 log divergences are
`umber2-alfh.8` (missing `show_context`) and the rest split across several
gaps this framing did not close: e-TeX's own "entering extended mode" banner
line, pdfTeX's `[<count0>...]` page-shipment marker, the bare `*` prompt
before an extra terminal read, and (on every terminal channel) `begin_job`
writing the terminal banner without INITEX's `(INITEX)` suffix that the log
banner and the oracle's own terminal both carry.

**A committed `expected/<case-id>.<channel>` file for an `xfail` channel is
understood to hold the reference engine's bytes, exactly like a `file`
channel's -- that is the one meaning a committed channel file has.** This
replaces the disposition's earlier meaning, under which the committed bytes
were Umber's own known-wrong output and the comparison was byte-identity
against that self-pin, indistinguishable from `file` except in name. An
`xfail` channel instead carries a `mismatch`: the first line at which Umber's
own output diverges from the committed reference, both sides rendered so a
divergent channel legibly records what TeX does and what Umber does instead
(using the literal `<end of channel>` for a side that runs out first).
Comparing an `xfail` channel then has three outcomes, mirroring the
case-level `expectation`'s own pass/xpass/changed-failure discipline:

- Umber's output still diverges exactly where and how `mismatch` says: pass.
- Umber's output now equals the committed reference bytes exactly: fail, as an
  xpass -- the pin no longer describes anything, so the fix must be recorded
  by promoting the channel to `file` and closing the bug, not left to a
  disposition that quietly keeps "passing" a bug that is gone.
- Umber's output diverges some other way -- a different line, or the same
  line with different text: fail, as a changed failure, reporting the pinned
  divergence next to the one now observed so a shift in behavior is never
  mistaken for the one `bug` names.

No committed case uses `xfail` for a stream channel today, so this is a
contract change with no corpus edit behind it yet: the next channel pinned as
`xfail` is the first to commit its reference bytes under the new meaning.

Regenerate the contract with:

```bash
scripts/regen-fixtures.sh --area command-semantic
```

It drives the same `tex_command_stream::semantic` module the gate does, so a
regenerated contract cannot describe a run the gate would not reproduce, and it
needs no reference engine. The emitted block matches `dprint`'s own shape and
the block replacement counts braces rather than matching a line, so the tool is
idempotent on its own formatted output.

### What The Clippy Gate Covers

One `cargo clippy` invocation lints one feature resolution, and Cargo unifies
features across every package the invocation selects. A whole-workspace
`--all-targets` run therefore always resolves `tex-command` and `tex-exec` with
`tex-state/testing` enabled, because every crate's dev-dependencies enable it.
No command of that shape can lint the resolution a released `umber` is built
in, so the gate runs a declared set of passes instead of one command.
`scripts/check-lint-passes.py` holds the declaration and runs them all:

- **union**: every workspace member, all targets, dev-dependency feature union.
  It selects `--workspace` rather than the default members because a test
  target only the exhaustive selection builds is still a target this repository
  compiles, and one selected by no pass is one the lint policy does not
  actually apply to (`umber2-johp.201`).
- **shipping**: every workspace member's lib and bin targets, no
  dev-dependencies. This is the resolution behind `cargo build -p umber`,
  `cargo run-dev -p umber`, and `cargo test -p umber --test it`. It no longer
  excludes `tex-command-stream`: that exclusion existed because
  `tex-command-stream` forced `observe` onto `tex-command` and `tex-exec`, and
  `observe` no longer exists (see
  [Cargo Feature Axes](cargo_feature_axes.md) §2.1). `tex-state/testing` is
  now the only thing the two passes differ by.

Together the passes lint every target of every workspace member in at least one
of the two resolutions.

What each feature name is allowed to mean, and which crate owns each
declaration, is a separate contract: see
[Cargo Feature Axes](cargo_feature_axes.md). It is what decides whether a new
feature belongs in a pass above or in `UNCOVERED_ENABLED_FEATURES`.

The declaration is verified rather than trusted. Each pass records the exact
feature set it expects Cargo to resolve for every workspace package and checks
it against Cargo's own `compiler-artifact` records; every feature a workspace
member declares must be enabled in a pass that lints its owner or be listed in
`UNCOVERED_ENABLED_FEATURES` with a reason; and every member must be linted
rather than merely compiled. Adding a feature, or changing which member enables
one, fails the gate until someone decides how it is covered. The features
listed as out of scope today are the opt-in profiling, `shadow`, `dvi-tools`,
and `reference-tools` configurations, several of which
`scripts/check-tools.sh` lints through `umber`. That hand-off is a deferral,
not an absence: `check.sh` ends by printing what `check-tools.sh` last did,
so "linted by `scripts/check-tools.sh`" is a claim a reader can check against
[Deferred Test Tiers](#deferred-test-tiers) rather than take on faith.

Denial happens in the script rather than through `-D warnings`: any diagnostic
from a workspace crate fails a pass, including one from a crate in dependency
position, which `-D warnings` never applied to. A known-dirty configuration is
quarantined per package and lint with an exact count and an issue id -- today
only `tex-command`'s nine `unused_variables` warnings in the shipping
resolution (`umber2-johp.200`). A quarantined lint is downgraded to warn for
its pass so the compilation survives long enough to report every diagnostic,
which costs no strictness because an occurrence in another package, or beyond
the recorded count, still fails the pass. Quarantined renderings are held back
so a green run prints no warning text, and the count is checked both ways:
fixing the warnings fails the gate until the quarantine entry is deleted, so an
exception cannot outlive its issue. `scripts/test-check-lint-passes.py` proves
each of these guards fails when it should, and the clippy gate runs it first.

Commands that execute Umber, including Cargo tests whose selected test enters
the engine, must run through the shared process-group guard when investigating
a hang or memory-growth failure. A targeted invocation is:

```bash
python3 scripts/run-umber-guarded.py \
  --timeout-seconds 120 --max-rss-mib 6144 --term-grace-seconds 5 -- \
  cargo test -q -p umber --test it TEST_NAME -- --nocapture
```

Use a smaller RSS or time limit whenever the fixture permits it. The guard
sums resident memory across the command's process group, sends TERM to the
whole group at either limit, waits no more than five seconds, sends KILL to the
whole group, reaps the leader, and fails if any group member survives. Exit 124
means a time or RSS limit fired; exit 125 means cleanup itself failed. Run
`scripts/test-run-umber-guarded.sh` to exercise the forced-timeout and RSS-limit paths. On macOS
the guard reads process-group membership and live resident size through `libproc`; on Linux it
reads `/proc`. These native paths avoid a global `ps` subprocess and run inside the development
sandbox without elevated permissions.
Compiler-only commands such as `cargo check`, rustfmt, and clippy do not need
the guard. The guard complements rather than replaces an explicit finite
engine expansion-fuel setting on the exercised `ExecutionContext`. Native
resource sessions accept the bounded `UMBER_ENGINE_FUEL` override; invalid or
hard-maximum-exceeding values fail before execution.

The explicit stepwise recent-arXiv validation tier is:

```bash
cargo build -q --profile test -p umber --bin umber
UMBER_ARXIV_FORMAT=/path/to/pdflatex.umberfmt \
UMBER_ARXIV_DISTRIBUTION=/path/to/verified/texlive-snapshot \
  scripts/run-stepwise-arxiv-census.sh
```

The runner is serial and gives every paper one process through
`scripts/run-umber-guarded.py`, with cumulative engine fuel, wall-time,
aggregate-RSS, process-group TERM-to-KILL, reap, and survivor enforcement.
It defaults to `scripts/pdftex-arxiv-recent-sample-100.tsv` and the matching
gitignored source archives under `third_party/arxiv-recent-sample-100`.
Before running, it derives each entrypoint and source identity directly from
the pinned archive bytes. Identity records the archive hash, normalized
member-manifest hash, member count, and entrypoint instead of hashing a mutable
directory. Each child receives a new exact archive extraction in an ordinary
temporary directory. Mutation, missing members, and generated extras fail the
hermetic tooling tests.
`RESOURCE_ENGINE_ACCEPTED` marks the transfer of accepted state to detached PDF
finalization in that process. A later map, encoding, PFB, PK, or PDF-lowering
failure therefore remains a finalizer outcome without recompiling the paper.
The TSV records both phase outcomes, replay telemetry, resource and engine time,
mutually exclusive accepted-run host phases, nested resolver/cache phases and
hit counts, estimated finalizer time, and guard status; failed rows retain
stable clusters.

The `profile.test` build is the optimized profile used by `cargo run-dev` and
shares its `target/debug/umber` artifact. A plain `cargo build` replaces that
path with the unoptimized development profile and is not a valid census binary.
The run identity records the exact binary path and hash. Row receipts retain
startup/format restore, engine, resource wait, VF lowering, font-usage, PDF
object/font embedding, image parse/copy, decode, transform, encode/cache,
serialization, materialization, and whole-run timings.

Each completed row has an atomically published JSON receipt under `rows/`.
Rerunning with the same binary, format, distribution manifest, sample, source
tree, limits, and mode rehashes its artifacts and skips it. Only an interrupted
row repeats. Changed identity or damaged evidence stops instead of mixing runs.
The explicit verified local distribution prevents fallback to the hosted pin.
The default is offline; set `UMBER_ARXIV_OFFLINE=0` for a warm cache-filling
run.

After a complete warm run, invoke the same results directory with
`UMBER_ARXIV_OFFLINE=1 UMBER_ARXIV_VERIFY_ONLY=1`. No child is launched: the
verifier rehashes immutable inputs and all durable row artifacts, then writes
`offline-verification.json`. This uses the native acquisition contract that an
acquired distribution object is digest verified and persisted in the
content-addressed cache before engine use, so attestation does not require a
second full compilation. `UMBER_ARXIV_LIMIT=1` selects the first row of the
recent sample.

Snapshot scaling has a separate explicit performance tier:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench snapshot_budgets
scripts/check-snapshot-budgets.sh
```

The Criterion command records the small/large latency rows. The script enforces
the low-noise latency ratio and requested-allocation retention budgets described
in [Snapshot Performance](snapshot_performance.md). Neither belongs in the
default cargo-test tier because its workload deliberately materializes large
input, page, mode, stream, hyphenation, provenance, and Unicode code-table
state.

Macro-invocation provenance has an assertion-bearing state performance tier:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench state_budgets provenance_memory/macro_long_run_arena_growth
```

Before timing, the benchmark expands 2,048 calls with 16-token bodies and
fails above 64 retained bytes per invocation. The charge includes archived
packed keys plus chunk and affine key-index metadata. Production admits at
most 1,048,576 record charges, a 64 MiB logical provenance-record budget;
excess diagnostic history degrades to unknown rather than aborting execution.

Classic BibTeX has its own release-only performance and persistence tier:

```bash
scripts/check-classic-bibtex-budgets.sh
```

It checks fixed cold-compilation, cache-hit, native-session, and browser
WASM-session ceilings against the committed classic corpus. The precise
workloads, retained-cache caps, pinned compatibility identity, extensions, and
Phase 9/epic exit audit are recorded
in [Classic BibTeX Compatibility Inventory](classic_bibtex_inventory.md).

Incremental edit mapping and convergence have a separate deterministic fuzz
tier:

```bash
scripts/test-incremental-fuzz.sh
```

The wrapper runs the ignored `tex-incr` 1,000-edit scripted test, comparing
the incremental DVI with a fresh cold execution after every revision. It stays
outside the default Cargo tier because of its intentionally long edit
sequence.

Whole-engine Gentle profiling has a separate persistent in-process runner:

```bash
scripts/profile-gentle.sh
```

It preloads external corpus and font inputs into a structurally shared memory
World, performs a warm-up, then repeats fresh engine sessions without
per-iteration temporary-directory or host-file staging. The script builds an
optimized symbolized binary and saves the Samply profile under
`target/profiles/`. Its incremental matrix separately verifies slow,
interaction, fast suffix-adoption, and break-dependency hlist-rebreak paths.
See [Profiling Umber with Gentle](profiling.md) for its controls and measured
boundary.

## Fixture Regeneration

`scripts/regen-fixtures.sh` is the sole live-reference rewrite path. It builds
`tools/fixturegen` for text/native and PDF fixture updates and `tools/refexec`
for DVI fixture updates. Its `--area pdf` mode requires pdfTeX 1.40.27 and
Poppler `pdftoppm` 25.08.0; its `--area fonts` mode owns the explicit live
`tftopl` cross-check and does not rewrite fixtures.

Its `--oracle tex82 --profile initex-eight-bit` and `--oracle etex26
--profile compatibility+extended-eight-bit` modes own pinned live reference
builds outside the correctness tier. Both reuse the hash-verified
TeX Live 2025 source cache offline, record the source/change/tool/platform and
executable identities under `target/`, and compare clean with instrumented
ordinary outputs. The e-TeX mode additionally verifies the distinct
compatibility and leading-`*` extended INITEX profiles, validates the complete
base schema-v1 event matrix in both, validates the focused expansion and
command-core extension matrix with compatibility exclusion, checks its
primitive-owner audit against canonical `etex.ch`, and repeats each base and
extension trace plus generated effect bytes deterministically.

The TeX82 workflow includes the committed `tex82/command-transitions-v1`
fixture gate; the explicit `--fixture` selector names the same gate. It
validates the contract-v1 manifest and hermetic bundle under
`tests/corpus/command`, audits every executable-matrix behavior against the
committed stream, focused sources, canonical citations, and ordinary-output
ownership, then requires the sources, stream, terminal, normalized log,
status, DVI, and generated effect to regenerate byte-for-byte. `tex-oracle`
unit tests consume that same committed bundle and the two pinned matrices
without a live TeX executable.
The focused source set includes separate legal and non-normal EOF programs so
the hermetic bundle distinguishes every TeX82 scanner-status recovery.

The `--oracle pdftex14027 --profile initex-etex-eight-bit` mode performs the
corresponding pinned pdfTeX 1.40.27 build. It gates DVI/PDF smoke artifacts,
the shared command
matrix, and a focused exact-eight-bit expansion/scanner matrix; proves the
549-primitive inventory as 391 shared TeX/e-TeX declarations plus 158 audited
pdfTeX additions with bidirectional primitive-to-matrix ownership; compares
clean and instrumented logs, status, DVI, PDF, and generated writes; parses
the smoke and state PDFs through the independent Hayro normalizer; and repeats
all three semantic traces plus state PDF projections byte-for-byte.

`--oracle all --profile canonical [--offline]` is the aggregate
cross-engine transparency gate. Before building, it validates the pinned
regeneration contract, exact source-manifest and fixture-audit hashes,
repository-owned inputs, event schema, canonical profiles, and committed
TeX82 fixture audit. It emits an uncommitted aggregate build record only after
all three workflows and the live TeX82 fixture comparison pass.
`--validate-only` performs the same hermetic identity, schema, and fixture
audit preflight without acquiring or building tools.

### Acquiring the pinned TeX Live 2025 source archive

`scripts/build-tex82-oracle.sh` fetches the pinned
`texlive-20250308-source.tar.xz` from the single host recorded in
`tests/trip-reference-manifest.txt`, `ftp.math.utah.edu`. That host fails TLS
verification on some networks:

```text
curl: (60) unable to get local issuer certificate
```

The same failure has been observed on `ctan.math.utah.edu` from
`scripts/fetch-conformance-inputs.sh`. The byte-identical archive is served by
the Chemnitz TUG mirror:

```text
https://ftp.tu-chemnitz.de/pub/tug/historic/systems/texlive/2025/texlive-20250308-source.tar.xz
```

Drop it at `third_party/texlive-source/`; the script's SHA-512 pin verifies it,
after which `--offline` works. The scripts are not mirror-aware, so a host that
fails verification currently blocks acquisition entirely rather than falling
back (tracked as `umber2-johp.170`).

See `tests/AGENTS.md` for the supported areas and cases, required tools,
copied support files, and validation performed after a rewrite.

The bibliography compatibility scaffold has one `bib-engine` Cargo
integration binary. It verifies all committed files below
`tests/corpus/bib/upstream-2.22` against a machine-readable SHA-256 manifest
that pins upstream commit `74252e608e5f8115375c532eb25416430a9f52eb` and the
Artistic-2.0 license. Its assertion-level xfail helpers cover exact strings,
bytes, deep values, and structured plus rendered diagnostics; a comparison
that unexpectedly matches is an XPASS and fails the test. Refreshing the
verbatim upstream input set is an explicit live-reference operation through
`scripts/regen-fixtures.sh --area bib`, never an ordinary Cargo-test action.
The same binary currently contains 1,275 assertion-isolated strict xfails for
51 foundation, input, graph, names, sorting, labels, uniqueness, output, and
tool-mode upstream files. Their Rust modules retain the complete pinned test
sources and exact assertion expressions for audit; subprocess-oriented output
tests record the equivalent in-process session status, byte-output, and
diagnostic expectations. The validation loop is expanded to 53 independent
tests so one XPASS cannot hide later validation assertions.

Classic BibTeX has a separate committed corpus under `tests/corpus/bibtex`.
Its manifest pins the TeX Live 2025 archive, `bibtex.web`, `bibtex.ch`, merged
Pascal, WEB2C-generated C/header, kpathsea and build configuration, exact
reference executable, inputs, status/history, BBL, BLG, and terminal bytes.
Its inventory assigns implementation and test owners to all 4 AUX commands,
10 BST commands, 3 BIB commands, 37 built-ins, 4 predefined symbols, and the
diagnostic, limit, branch, and upstream-test families. Ordinary tests audit
those committed bytes and owners only. The explicit
`scripts/regen-fixtures.sh --area bibtex` route builds and identity-checks the
pinned reference, executes it in an empty fixed-locale environment, refreshes
the outputs atomically, and reruns the hermetic audit.

The LaTeX format builder is a separate deterministic integration tier:

```bash
scripts/build-latex-format.sh --engine latex
scripts/build-latex-format.sh --engine pdflatex
```

Both modes verify that two clean format builds are byte-identical and that a
source-loaded smoke document exactly matches the corresponding format-loaded
document. The LaTeX mode compares DVI; the pdfLaTeX mode compares PDF. The
builder reads the common and mode-specific TeX Live input closure from
`tests/latex-source.lock`; its pdfLaTeX configuration is pinned locally in
`tests/latex/pdftexconfig.tex`. Generated formats and comparison artifacts
remain under `target/` rather than becoming repository fixtures.
All builder-started Umber and format-cache subprocesses reuse
`scripts/run-umber-guarded.py` with finite engine fuel, aggregate process-group
RSS and wall-time ceilings, and TERM-to-KILL/reap enforcement. Compiler-only
work remains outside that guard. Tune the bounded builder through the
`UMBER_LATEX_FORMAT_ENGINE_FUEL`, `UMBER_LATEX_FORMAT_MAX_RSS_MIB`, and
`UMBER_LATEX_FORMAT_TIMEOUT_SECONDS` variables rather than writing a separate
watchdog.
With `--publish-input-closure`, format metadata schema 2 also records the
canonical sorted request keys derived from that already verified trace. The
production snapshot builder uses this mode for both engines, stages local
configuration inputs into a pinned auxiliary root, and requires two complete
schema-3 publications to be byte-identical. Publisher tests cover closure
canonicalization, duplicate and size rejection, missing-key corruption, and
deterministic output without invoking live TeX tools.

## Committed DVI Corpora

The hand-authored distribution contract fixtures under
`tests/corpus/distribution` are consumed directly by both the dependency-free
`umber-distribution` Rust tests and authored JavaScript schema tests. They pin
strict manifest round trips and identical ordered acquisition jobs and typed
misses without network or TeX tooling.

The DVI corpora under `tests/corpus/dvi`, `tests/corpus/page`,
`tests/corpus/math`, `tests/corpus/align`, and `tests/corpus/leaders` commit TeX
source files plus `.expected.dvi` reference fixtures. The default `umber` cargo
tests run every `.tex` case in those areas against the committed DVI fixtures
without invoking live reference tools.

DVI regeneration runs the live reference engine through `tools/refexec`,
copies the pinned local CM TFMs and area support files, uses INITEX for the math
corpus, and rewrites raw reference DVI only when the existing
preamble-comment-only comparison detects a change.

Only the math corpus uses `--ini`; its sources declare their own `\catcode`
preamble because INITEX leaves `{`, `}`, `$`, `&`, `#`, `^`, and `_` as
`other_char` (tex.web §232). Every other area is regenerated against a
format-loaded reference engine, so `umber run` matches it by synthesizing that
part of the format prelude in `umber::prepare_run_stores` rather than in the
INITEX code-table defaults. `umber lex-dump` and `umber expand-dump` report the
same format-loaded state, and their committed corpora rely on it.

## Committed PDF Corpus

`tests/corpus/pdf` commits minimal primitive-only sources, pinned reference
PDFs, deterministic Umber PDFs, normalized catalog/page/resource/content
structure, exact 72-dpi grayscale PGM renders, and renderer/hash attestations.
Synthetic PDF parser and importer inputs use the dependency-free
`test_support::pdf_fixture` classic-xref writer. It deterministically checks
indirect-object offsets and stream lengths while leaving complex object-stream
syntax to committed externally generated fixtures.
The `form_xobjects` case additionally canonicalizes decoded Form XObject
dictionaries and content operations, pins nested h/v/math placement and reuse,
and drives retained-session artifact/position/snap replay coverage.
Regenerate it only with `scripts/regen-fixtures.sh --area pdf` or
`scripts/regen-fixtures.sh --case pdf/<case>`.

Regeneration resolves object references and removes only byte-layout and
volatile metadata differences before comparing structure. It then renders
both PDFs with pinned Poppler and requires exact dimensions and pixels. The
ordinary cargo test invokes neither external tool: it rebuilds the exact Umber
bytes, normalizes the committed reference and current output, and verifies the
SHA-256 chain connecting both committed PDFs to the equal raster.

The independent host-tool gate is versioned as
`scripts/check-pdf-external.sh`. Its qpdf 12.3.2 matrix checks representative
classic trailers, all three object-compression policies, imported PDF and
raster/alpha/DCT images, Type 1/TrueType/PK/subset/tagged fonts, annotations,
forms, and navigation actions. Separately, Poppler 25.08.0 re-renders every
committed Umber PDF and compares it with the pinned PGM (exactly for ordinary
cases and with gray-value delta two for font cases); font extraction must also
match the committed UTF-8 bytes. Run `scripts/check-pdf-external.sh --local`
for development. A missing tool produces an explicit skip only in this mode;
an installed tool with the wrong version still fails. CI and release jobs must
install the pinned qpdf and Poppler versions and run
`scripts/check-pdf-external.sh --ci`, where missing tools and every validator
warning are fatal. `UMBER_PDF_VALIDATOR`, `UMBER_PDF_RENDERER`, and
`UMBER_PDF_EXTRACTOR` may select explicit executable paths.

## End-to-End Conformance Gate Contract

The four byte-exact end-to-end DVI gates (`story`, `gentle`, `trip`, `etrip`)
compare Umber's assembled DVI against an oracle produced by a real reference
engine. Those `tests/corpus/e2e/<name>.expected.dvi` oracles derive from
third-party documents, are gitignored on purpose, and must never be committed.
That licensing decision stands. Its consequence must not: an absent oracle used
to make each gate print `skipping ...` and return, and libtest discards a
passing test's captured output, so the notice was invisible without
`--nocapture`. A fresh worktree therefore reported a clean suite while the
epic's flagship byte-exact Story DVI parity result never executed.

The contract now is:

- **A run that skips a gate is never indistinguishable from a run that passes
  it.** When every required asset is present, the gate writes a confirmation
  line to the process's real stderr handle:

  ```text
  conformance gate `story`: running against tests/corpus/e2e/story.expected.dvi
  ```

  That write bypasses libtest's output capture, so it appears without
  `--nocapture`. Grep for it to prove a gate executed.
- **Absence fails, loudly and actionably.** The gate panics with a report
  naming every missing asset, why it is missing, and the exact commands that
  materialize it.
- **Skipping is structurally unreachable from a gate body.** Every gate is
  registered in `assets::GATES` in
  `crates/umber/tests/it/e2e_conformance/assets.rs` and reaches its assets only
  through `assets::with_gate`, which has no caller-visible skip path. Two
  meta-tests hold that shape:
  `conformance_gate_registry_matches_gitignore` requires the registry and the
  gitignored `/tests/corpus/e2e/*.expected.dvi` entries to be in exact
  correspondence, and `conformance_gate_registry_is_reachable` requires every
  registered gate to have a real `with_gate` call site. Adding a sibling gate
  with a private presence check of its own fails both.
- **The single non-failing absence path is explicit.** Setting
  `UMBER_CONFORMANCE_ORACLES=optional` downgrades absence to the same report
  written to real stderr, again uncapturable. Any other value for that variable
  is rejected rather than silently treated as "required". A run that sets it has
  forfeited the byte-exact parity results and must not be reported as clean; do
  not set it in CI or in agent runs.

`scripts/check-and-test.sh` preflights the oracles before starting the workspace
gate and warns that absent ones will cause failures, not skips. Its list is read
from `.gitignore`, the same single source the registry meta-test binds to, so
the preflight cannot go stale when a gate is added.

The contract is usable from an isolated linked worktree without a setup step.
`test_support::native_assets::provision` resolves the primary checkout from
Git's shared worktree metadata and materializes missing files from the explicit
`tests/native-test-assets.lock` allowlist, on first use, from inside the suite.
The allowlist contains only the four oracles and their declared corpus,
hyphenation, TFM, and TRIP/e-TRIP file dependencies; it cannot select a
directory. Every source and destination is verified against its committed
SHA-256, and copies are used rather than links so a worktree cannot mutate the
owning checkout's evidence.

`crates/umber/tests/it/e2e_conformance/assets.rs`'s `with_gate` is the single
call site, which is the same choke point that already makes an absent oracle
impossible to confuse with a passing gate. When the primary checkout itself
lacks an asset there is nothing to copy from, and the failure names the missing
paths and points at `scripts/setup-conformance-tests.sh`.

Every source and destination must match its committed SHA-256. Provisioning
uses an independently verified temporary copy followed by atomic rename, not a
symlink or hard link, so code running in one worktree cannot rewrite the
primary checkout's evidence through the provisioned path. Existing mismatched
files are rejected rather than replaced, missing primary assets produce an
error naming that checkout and the setup command, and successful copies remain
gitignored so `git status` stays clean. A primary-checkout run never searches
another cache or downloads anything; it reports its exact missing allowlist and
requires `scripts/setup-conformance-tests.sh`.

Story and Gentle additionally verify their oracle against the
`expected_ref_dvi_sha256` pin in `tests/corpus-manifest.txt` inside
`parity_harness::run_named_fixture_document`, so a stale or foreign oracle fails
with a hash-drift message rather than a confusing DVI mismatch. TRIP and e-TRIP
do not have a normalized-DVI manifest pin inside the Rust harness. Their raw
bytes, like all assets copied between worktrees, are nevertheless pinned by
`tests/native-test-assets.lock`; an intentional regeneration therefore requires
an audited lock update rather than silently distributing one checkout's changed
oracle to every linked worktree.

## External Document Corpus

External document inputs live outside committed fixtures. The line-oriented
`tests/corpus-manifest.txt` pins support files and documents by URL, fetched-byte
SHA-256, license determination, and redistributability flag. Runnable documents
also select a format source and pin the reference DVI SHA-256 after DVI preamble
banner normalization.

`scripts/setup-conformance-tests.sh` builds `tools/corpus-sync` to fetch or
verify those inputs under gitignored `third_party/corpus/`, then acquires the
remaining local support files and generates all four end-to-end DVI oracles.
Cached hash matches are a no-op. Fixture regeneration pins
`SOURCE_DATE_EPOCH=1783604160` and `FORCE_SOURCE_DATE=1` so date-sensitive
documents have stable DVI body bytes. Once setup completes, the conformance
tests consume only local files and require no network access.

Full external-document DVI parity is exposed as local-oracle-backed Cargo
integration tests:

```bash
cargo test -p umber --test it e2e_conformance_story -- --nocapture
cargo test -p umber --test it e2e_conformance_gentle -- --nocapture
```

Populate the external inputs and all Story, Gentle, TRIP, and e-TRIP DVI oracles with
`scripts/setup-conformance-tests.sh`. The generated `.expected.dvi` files are
gitignored licensing-sensitive derivatives and are not repository fixtures.

The shared `parity-harness` library stages inputs, calls the Cargo test's in-process Umber
runner, and byte-compares its normalized DVI with the local `tests/corpus/e2e`
oracle. Each document names a manifest-pinned
`format_source`; the harness stages that source, the document, hyphenation
input, and required TFMs, then feeds Umber a wrapper that inputs the format
source before the document through the ordinary input path.

The Story and Gentle callbacks also scan fixed-width provenance records after
execution, print invocation count, macro-attributed retained bytes,
bytes-per-invocation, and total provenance retention, and fail above the same
64-byte per-invocation budget as `state_budgets`. This scan is outside macro
expansion and therefore does not require profiling-only hot-path counters.

This follows TeX82's ordinary `start_input` stack behavior (sections 23 and
29). Format dumping is a terminal INITEX cleanup operation (sections 46, 50,
and 51), not a way to continue into the document. The pinned modern
`plain.tex` contains no `\\dump`, so it can be loaded directly.

On fixture-hash drift, Umber failure, or mismatch, the harness writes a triage
bundle under `target/conformance-triage/<doc-name>/` with byte context,
page-limited dvitype-style disassemblies, a unified diff, tracing logs, and a
summary naming the divergent page and opcode when recoverable from DVI
backpointers. The `cargo test -p parity-harness self_test_bundle_pinpoints_page_and_opcode`
command exercises the bundle writer with synthetic DVI.
`scripts/regen-fixtures.sh --case e2e/story` and `--case
e2e/gentle` verify the manifest-pinned normalized reference hash before
rewriting either fixture.

### Canonical Story and Gentle Regression Gates

`e2e_conformance_story_canonical` and
`e2e_conformance_gentle_canonical` in the same
`crates/umber/tests/it/e2e_conformance.rs` check canonical/reference DVI
parity: the canonical `tex-command`/
`CanonicalEngineSession` architecture's assembled DVI for `story.tex` and
`gentle.tex` must remain byte-identical to real pdfTeX's output, normalized
only the same way the legacy conformance tests already are. They are kept
alongside, not instead of, their legacy `EngineSession` gates while the
migration is in progress. Each shares its exact staged fixture directory
(`parity_harness::run_named_fixture_document`, the same
`plain.tex`/document/`hyphen.tex`/TFM staging the legacy in-process runner
consumes) and its registered gate, so both reach their assets through
`assets::with_gate` and neither can skip silently. See the
[End-to-End Conformance Gate Contract](#end-to-end-conformance-gate-contract)
above for what an absent oracle does.

```bash
cargo test -p umber --test it e2e_conformance_story_canonical -- --nocapture
cargo test -p umber --test it e2e_conformance::e2e_conformance_gentle_canonical -- --exact --nocapture
```

The Gentle oracle is the existing 263424-byte real-pdfTeX artifact, SHA-256
`04f86e97e8264f9b8ce35dc1e9df27f2b075ca85365af71acc5fe1478399866b`.
The canonical gate now matches that oracle byte-for-byte and therefore runs in
the routine native suite. Gentle remains a byte-exact DVI conformance gate,
not an automated differential-tracer fixture: the tracer's structural tests
admit only committed microfixtures and synthetic fixtures, and do not load
Gentle or another full document.

Unlike the legacy runner (`EngineSession` over `ExecutionContext`/
`InputResolver`/`FontResolver`), this test drives
`umber::CanonicalEngineSession` directly: it seeds the staged directory into a
memory `World` (via the shared `staged_world` helper both runners now use),
installs the canonical primitive tables with
`CanonicalEngineSession::tex82_initex`, whose command core owns the expandable
table and composes it with `tex_exec::install_unexpandable_primitives` (matching
`examples/first_failure_locator.rs`'s proven-working setup rather than the
CLI's `prepare_run_stores`), registers the staged job wrapper as an authored
root, and answers `\input`/font resource suspensions from a `StagedDirResourceHost`
that reads the same staged files the legacy resolvers do. It does not perform
the legacy test's macro-invocation-provenance budget check: that budget is a
legacy `EngineSession`/`ExecutionContext` observation that does not yet have a
canonical-session equivalent (see `umber2-johp.75`).

Under the `cargo test --tests`/`profile.test` (`opt-level = 1`) build this test
uses, Story's canonical run is fast (a few seconds), not the roughly 50-second
debug-build cost tracked separately in `umber2-johp.74`; the whole gate adds
on the order of 2 seconds to `cargo test -p umber --test it`'s wall time
alongside the other e2e cases.

On a mismatch it fails through the exact same `parity_harness::compare_dvi_files`
byte-identity contract as the legacy test, reporting the divergent page and
DVI opcode and writing a triage bundle under
`target/conformance-triage/story.tex/`. This was verified directly: temporarily
corrupting one assembled byte before the DVI comparison made
`e2e_conformance_story_canonical` fail with an exact byte/page/opcode mismatch
while `e2e_conformance_story` (legacy) kept passing; reverting the corruption
restored both to green. See the diagnosis order in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#2-diagnosis-order)
for the differential-tracer/first-failure-locator recipe to use once this gate
actually fails on a real regression.

## Canonical Command-Core Diagnostics

Two tools locate a `umber2-johp` command-core divergence or failure. This
section describes what each tool is, what it requires, and what it prints.
For the order to run them in, why the retired Umber implementation is never
an oracle, and what to do with the result, see the diagnosis order in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#2-diagnosis-order).

### Differential Tracer

```bash
cargo run -q -p tex-command-stream -- --repository . --max-divergences 100000
cargo run -q -p tex-command-stream -- --repository . --realign-window 128
cargo run -q -p tex-command-stream -- --repository . --ungrouped
```

The default budget (`DEFAULT_MAX_DIVERGENCES` = 20) saturates on `gentle`, so
a run without `--max-divergences` returns `PARTIAL` (exit `2`) and totals that
are floors. Pass a budget large enough to exhaust every fixture whenever the
totals are going to be compared against anything.

Run this from the repository root. It replays the committed
`tests/corpus/command/tex82` fixture registry and the separately pinned
`tests/tex82-oracle/geometry.tex` microfixture through the instrumented command
boundary and compares the translated `tex-oracle` event streams against their
expected traces. The geometry source selects schema v2 and compares its
detached hpack, vpack, and shipout projection; ordinary command fixtures remain
schema v1. The run is fully hermetic (no external corpus, distribution, or live
TeX tool required) and never invokes a reference engine.

The native correctness suite runs this committed-microfixture comparison and
requires `CLEAN`; its `committed_tex82_command_traces_are_clean` test uses the
committed-only runner, which validates the fixture inventory before replaying
it. An absent or drifted committed fixture therefore fails explicitly rather
than making the gate look clean. Selection also enforces a structural
microfixture footprint: at most 64 source files, 64 KiB of combined source,
and 50,000 ordered events per fixture. A registry entry beyond any bound fails
with its observed footprint, every limit, and the manual command to use
instead. This is deliberately name-independent, so accidentally registering
any full document is rejected rather than relying on a list of known document
names. The generated document-trace tree is loaded only by the explicit CLI
runner, so the routine suite does not replay Plain, Story, Gentle, TRIP, or any
other full document. The committed geometry microfixture is deliberately
font-independent and covers explicit packaging, paragraph line packing, an
explicit shipment, and end-of-job page-builder shipment. Controlled hpack and
shipout mutations gate `geometry_mismatch` diagnostics, including both
expected and actual signed scaled-point values.

It reports a ranked WORKLIST, not just the first divergence:

Every run that happened prints its report, ending with a `VERDICT:` line
naming the outcome and the exit status carrying it. The status answers one
question: whether the printed totals are the whole truth.

- Exit `0` (`CLEAN`): every registered fixture was compared to exhaustion and
  none diverged. The report is still printed, because a check that prints
  nothing cannot be told apart from a check that did not run.
- Exit `1` (`DIVERGED`): every registered fixture was compared to exhaustion,
  so the divergence total is exact. Prints up to `--max-divergences` ordered
  divergences (default
  `DEFAULT_MAX_DIVERGENCES` = 20) per fixture, in stream order across every
  registered fixture, collapsed into one entry per root site (see "Grouped
  worklist" below). Two entry shapes share this ordered list:
  - A stream mismatch: the expected event, the actual observed event, and
    source context, labeled with a cheap structural `kind` (for example
    `command_identity_mismatch`, `command_operand_mismatch`,
    `event_kind_mismatch`, `mutation_mismatch`, `stream_truncated_early`) so
    same-kind entries can be batched without re-running the engine. The
    header line also carries the resynchronization the comparator applied and
    the cascade that resynchronization absorbed:

    ```text
    fixture <name> diverged at event 11375 (observed event 11385)
      [event_kind_mismatch] (resync: 1 oracle event(s) dropped by Umber;
       suppressed 32 cascade event(s))
    ```

    The observed index is printed only once it has drifted from the oracle
    index. See "Stream alignment" below for what each resync means.

    Both rendered events are truncated to a bounded number of characters, so
    a long payload -- a macro body, a token register, a mutation value --
    can differ past the cut and print two identical-looking sides. When that
    happens the entry carries one extra pair of lines naming the character
    offset where the two renderings first differ, with a window of context
    around it:

    ```text
    first difference at character 4325, past the truncation above:
      expected: …, OracleToken { … "mac_param" … }, OracleToken { … }] })
      actual:   …, OracleToken { … "mac_param" … }, OracleToken { … }] })
    ```

    It is text-level rather than schema-aware, so it works for every event
    kind without enumerating payload fields, and it is emitted only when the
    truncation actually hid the difference.
  - A contained replay failure (`engine panicked` or `replay failed`): a
    command-core `ExecError` or a Rust panic during that fixture's replay is
    caught (`catch_panic`/`ReplayFailure`) and reported as its own ordered
    entry with the fixture, the event index it occurred after, and the
    failure's message (a panic's message and source location, exactly as
    the default panic hook would have printed -- `RUST_BACKTRACE=1` still
    produces a full backtrace on stderr). It does not abort the run: fixture
    replay continues afterward for any fixture ordered after it, up to the
    divergence budget. A panic outside a fixture's replay proper (fixture
    loading, suite/contract validation, argument parsing) is not contained
    and still aborts the process with the ordinary Rust panic exit code.
- Exit `2` (`PARTIAL`): the run did not compare everything it registers, so
  every printed total is a LOWER BOUND rather than a total, and a total of `0`
  would not mean convergence. Two conditions earn it, and the verdict names
  which applied and to which fixtures: a registered document trace that is not
  generated on this checkout (regenerate with
  `scripts/build-tex82-document-traces.sh`), and a fixture whose
  `--max-divergences` budget stopped its comparison early (raise the budget).
  This status exists because `umber2-johp.168` found a fresh checkout
  reporting `4 ordered divergence(s)` where the true count was `160`: three of
  the four registered fixtures had never been compared, and nothing in the
  exit status distinguished that from convergence. Never rank or dispatch from
  a partial worklist.
- Exit `3`: the run could not be performed at all -- a usage error, an
  unreadable suite, or a document registry inconsistent with its committed
  pin. It is kept distinct from exit `1` so "the tool refused to run" is never
  read as "the tool ran and produced this worklist".

See `docs/command_semantic_fixtures.md` and `docs/alignment_brace_semantics.md`
for the fixture registry and event schema this replays and compares against,
and `tools/AGENTS.md` for what the tool does and does not do.

#### Grouped worklist and run accounting

One defect reaches the ordered worklist once per source position it recurs
at, so a preload loop that assigns the same wrong meaning forty-eight times is
forty-eight entries that are identical apart from their `SourceLocation`. The
report collapses those into one entry each. The run opens with two separately
labeled totals and a per-fixture accounting:

```text
759 ordered divergence(s) in 200 root site(s):
  divergence(s): what the comparator found. Grouping does not change this
    number; it is the one to compare against historical totals.
  root site(s): the entries below, one per group of divergences that are
    identical after erasing source positions and nothing else. Every
    divergence is in exactly one group; none is dropped, sampled, or
    truncated. Pass --ungrouped for one entry per divergence.
per fixture, in replay order:
  tex82/command-transitions-v1  1 divergence(s) in 1 root site(s), first at oracle event 5892
  tex82/document-plain-v1       0 divergence(s)
  tex82/document-story-v1       0 divergence(s)
  tex82/document-gentle-v1      758 divergence(s) in 199 root site(s), first at oracle event 102452
```

The two numbers answer different questions and only one of them is comparable
against a historical figure.

- **divergence(s)** is what the comparator found. Grouping does not change it,
  and every "N entries" figure recorded in `umber2-johp` before grouping
  existed is this number. Compare a before/after fix against _this_.
- **root site(s)** is how many entries the report prints. It is a triage
  metric: it says how many distinct things a coordinator has to dispatch.

Grouping is presentation only. The ordered comparison, the entry order, and
the divergence count are identical with and without it, and `--ungrouped`
prints the one-entry-per-divergence worklist -- byte-identical to the
pre-grouping report body -- so the grouped view can always be checked against
the list it summarizes. Both views print both totals.

Two divergences are the same root site when they are equal after erasing every
source position and _nothing else_. `group::positionless_event` is an
exhaustive match over the `tex-oracle` event schema -- `CanonicalCommand`'s
location plus every `OracleToken` reachable through commands, recovery events,
macro arguments, token lists, scanner results, mutations, diagnostics, and
effects -- so adding a schema variant fails to compile rather than silently
carrying a position, or a payload, into the key. Everything else separates two
entries: differing operands, differing token payloads, a differing repair
shape (three dropped oracle events is not twenty-one), a differing fixture,
and a macro call's token-list address. `Repair::AnchorResync` compares by
anchor _kind_ with a line-anchor's line erased, since that line is a position
like any other. The bias is deliberate: under-merging leaves a longer
worklist, but over-merging hides a second defect behind the first.

A grouped entry prints its count, renders its first occurrence exactly as the
ungrouped worklist renders it, and then names every occurrence:

```text
[75] x109 fixture tex82/document-gentle-v1 manifest=... diverged at event 141407 ...
  expected: Input(InputEvent { transition: Push, reason: TokenList, name: "every_par" })
  actual: Command(CommandEvent { delivery: Raw, command: CanonicalCommand { ... } })
  context: source=gentle.tex; input_level=5803; position=0
  recurrence: 109 exact occurrence(s) of this root site, 1196 suppressed cascade event(s) in total;
    the entry above is the first. Every occurrence, by oracle event index:
    141407, 141450, 141486, 141551, 141619, 143260, ...
```

The oracle event index list is printed whole and never elided: an entry that
stands for a hundred occurrences has to let the agent dispatched on it reach
all hundred. A group's suppressed cascade is the sum over its members. A
single-occurrence entry prints `x1` and no recurrence block.

Two bounds the report used to leave a reader to infer are now printed.

- A fixture whose `--max-divergences` budget was reached while events remained
  prints `BOUNDED:` under its accounting line, naming the flag and saying that
  comparison of that fixture stopped there. See "What the budget counts"
  below for the unit, which grouping does not change.
- A document trace that is not generated on this checkout is listed as
  `not compared -- trace not generated on this checkout`, in addition to the
  stderr notice, so a fixture that never ran cannot read like a clean one.
  Compared fixtures with no divergence are listed with an explicit `0`.

Both bounds also decide the exit status, because a reader who checks only
`$?` is exactly the reader a partial run misleads. Either one makes the run
`PARTIAL` (exit `2`) no matter what the divergence total says, and the report
closes with a `VERDICT:` line repeating the outcome, the status, and which
fixtures were left short:

```text
VERDICT: PARTIAL (exit 2) -- this run did not compare everything it
  registers, so 1 ordered divergence(s) is a LOWER BOUND, not a
  total, and a total of 0 would not mean convergence.
  never compared (3): plain, story, gentle
  generate the missing traces with scripts/build-tex82-document-traces.sh
```

A partial run also withdraws, in the header itself, the instruction the
complete-run header gives. The header is where a reader takes a number from,
so leaving `it is the one to compare against historical totals` printed over a
floor would be this epic's recurring defect in miniature -- a number labeled
as something it is not. A bounded or incomplete run prints instead:

```text
20 ordered divergence(s) in 7 root site(s):
  LOWER BOUND: this run stopped short of comparing everything it registers
    (the per-fixture accounting below and the VERDICT line at the end name
    which fixtures and why). Every total above is a floor, not a total, and
    none of them is comparable against a historical figure.
  divergence(s): what the comparator found before it stopped short.
    Grouping does not change this number; the bound above does.
```

An exhaustive run's report is byte-identical to what it was before that
annotation existed, so no figure measured from an exhaustive run moved.

#### What the budget counts

`--max-divergences N` bounds **ordered divergences**, per fixture. It bounds
neither root sites nor printed entries, and it never has. Since the worklist
began printing one entry per root site the three are different quantities: a
bounded run of `N` divergences prints at most `N` grouped entries and usually
fewer, and prints exactly `N` under `--ungrouped`. Re-basing the budget onto
root sites was considered and rejected (`umber2-johp.207`):

- It would bound nothing. The budget exists so one long fixture cannot produce
  an unbounded walk and an unbounded report, and the case it was introduced
  for is a single structural defect recurring without end. That defect is
  _one_ root site however many times it recurs, so a budget of 20 root sites
  would walk the whole fixture and print a recurrence index list thousands
  long -- the outcome the budget prevents today.
- It would move the ambiguity, not remove it. Budget and printed entry count
  agree exactly under `--ungrouped` today and would stop agreeing there. No
  unit equals the printed entry count in both views, so that equality is not
  an available invariant.
- It would make the comparator depend on the presentation layer. Grouping is
  documented as changing only how the worklist prints, never what is compared
  or in what order; a root-site budget would let the grouping projection
  decide where the comparison stops, so the two views would compare different
  amounts of the stream.

The invariant kept instead is that every number names its unit where it is
printed. A bounded fixture's `BOUNDED:` notice says the budget counts ordered
divergences and neither root sites nor printed entries, and names its
divergence total and its root-site total as floors:

```text
tex82/document-gentle-v1  20 divergence(s) in 7 root site(s), first at oracle event 204839
                          BOUNDED: --max-divergences 20 counts ordered divergences; it
                          counts neither root sites nor printed entries. Comparison of this
                          fixture stopped at 20 of them, so its 20 divergence(s) and
                          7 root site(s) above are both floors: more of each exist
                          beyond its last entry.
```

One divergence per fixture sits outside this budget: the contained replay
failure (`engine panicked` / `replay failed`). It is at most one entry, names
a concrete `ExecError` or panic site, and must not be crowded out by the
twentieth consecutive mismatch of an already-reported structural defect. A
bounded fixture's divergence total can therefore exceed the budget by one, and
when it does the notice says so rather than leaving the arithmetic to look
like an overrun:

```text
Its contained replay failure is reported outside the mismatch
budget, which is why 21 is more than the budget of 20.
```

#### Stream alignment

The comparator (`tools/tex-command-stream/src/compare.rs`) treats the pinned
oracle stream and the observed stream as two sequences to be aligned, not as
two index-parallel arrays. Index-aligned comparison is only correct while
both streams agree on how many events each delivery produces; one dropped or
extra event otherwise turns every later index into a mismatch, and one root
defect fills the whole per-fixture budget with entries that say nothing new.

Every event splits into an alignment KEY and a PAYLOAD.

- The key is identity: the event kind, the canonical command identity
  (command name, control-sequence spelling, raw/expanded delivery) and its
  source position, and every structural transition -- input push/retire/stop,
  condition push/branch/pop, alignment transitions, token-list
  splice/complete, macro argument vs. activation.
- The payload is content: operands, scanner results, mutation keys and
  values, align state, token lists, diagnostic arguments.

A command's source position is part of its identity because long runs of
like-catcode characters are otherwise indistinguishable, and a shifted stream
could confirm a realignment against the wrong occurrence.

From that split the comparator produces one of these resyncs per entry.

- `payload differs, streams stay aligned` -- same key, different payload. A
  content-only defect; nothing was skipped and nothing cascades.
- `N oracle event(s) dropped by Umber` -- the oracle emitted N events Umber
  never produced.
- `N extra Umber event(s)` -- Umber emitted N events the oracle never
  produced.
- `N oracle event(s) replaced by M Umber event(s)` -- a short edit run; one
  replaced by one is an ordinary substitution.
- `structural: ... rejoined at <anchor> after skipping ...` -- nothing
  confirmed inside the window; the anchor fallback rejoined the two streams.
- `structural: ... no shared anchor; comparison of this fixture stopped here`
  -- neither the window nor an anchor confirmed a repair.
- `one stream ended with N event(s) remaining in the other` -- one stream ran
  out first.

On a key mismatch the comparator runs a wavefront search confined to a window
of events on each side, visiting candidates in ascending edit distance (and,
within one distance, ascending oracle skip) so the smallest repair wins
deterministically. A candidate is accepted only after a run of consecutive
key-equal pairs confirms it. When fewer than that many pairs remain before
the end of a stream, all remaining pairs must agree; skipping both streams to
their ends with no agreeing pair at all is a fork, not a repair.

If nothing confirms inside the window, the divergence is structural and one
anchor resync is attempted: both streams are scanned forward over every
high-salience boundary -- an input-stack push/retire/stop, or the first
delivery attributed to a new source line -- inside the scan bound, and the
streams rejoin at the most identifying shared boundary that carries the same
confirmation. If that also fails, comparison of that fixture stops and says
so. The bias is deliberate: cascade noise is visible, but a real defect
hidden behind an over-eager realignment is not.

The two anchor kinds are not equally identifying, and the search ranks them
in that order rather than by cost. A shared source line names the same
physical position in the same named file on both sides, so it is evidence
that the streams are at the same point in the _document_. An input push names
only the shape of a boundary: every macro activation in a run carries the
identical `Push/Macro macro` key and every backup the identical
`Push/Backup backup`, so a shared one is evidence of nothing beyond "both
sides pushed something". A shared line therefore wins over any anonymous
boundary in reach, however much cheaper that boundary is; anonymous
boundaries are used only when no line is shared inside the scan.

Within one class, least total skip decides, and that is not a tie-break
detail either. Rejoining at a costlier shared anchor lands the streams on a
boundary they agree at only locally, and the next real key mismatch then has
no shared anchor left inside the scan, so an inherited over-costly rejoin is
reported as a structural fork that stops the fixture. Anchors are enumerated
by oracle offset, and the cheapest pair is frequently not the first one
visited: a distant oracle anchor paired with an immediate observed one
undercuts a nearby oracle anchor paired with a far observed one.

This is not a global minimum-edit-distance diff, and deliberately so. Myers
is `O(ND)` and Gentle's trace is over 100 000 events; worse, a global minimum
would happily pair oracle event 700 with observed event 40 000 when that
minimizes total edits. Every search here is local, bounded, and paid at most
once per reported divergence, so the run stays linear in the streams.

`suppressed N cascade event(s)` counts the mismatches plain index-aligned
comparison would have reported over the stream region this entry covers --
from this entry's oracle index up to the next reported entry's, or to the end
of the streams for the last entry -- not counting the entry itself. It is the
cascade the entry stands in for, and it is how to tell one root site from
many: as of this writing `gentle` reports 13 550 divergences in 2297 root
sites where index-aligned comparison would report hundreds of thousands, and
`plain` and `story` are clean.

#### Alignment tunables

Three flags bound the search. Widen them when a suspected repair is larger
than the defaults; narrow them to prove a reported realignment is not an
artifact of an over-generous bound.

- `--realign-window` (default 64) -- half-width of the wavefront search, in
  events per stream.
- `--realign-confirm` (default 8) -- consecutive key-equal pairs required to
  accept a realignment.
- `--anchor-scan` (default 4096) -- events scanned forward on each side by
  the structural anchor fallback.

A window of 64 costs `O(window^2)` key comparisons in the worst case, paid at
most once per reported divergence, and comfortably spans every repair shape
this epic has produced (a missing backup push, a duplicated raw/expanded
delivery pair, a macro activation expanded one level too far) while staying
far below the distance at which a confirmed match would be coincidence rather
than the same point in the document. A confirmation run of 8 is far more than
the two or three events that repeat by chance inside a run of like-catcode
characters, and small enough that a genuine repair immediately followed by a
second independent defect still confirms, leaving the second defect to be
reported on its own. The anchor scan is only reached once the window search
has already failed, so 4096 events -- roughly a page of document activity --
is generous on purpose. It is the only bound on the fallback's reach: every
anchor inside it is a candidate, so widening the flag really does widen the
search. A dense trace region crosses dozens of input-stack boundaries in a
few hundred events, so any secondary cap on the anchor count would quietly
shorten this flag to a fraction of its stated reach.

All three flags take a positive integer and reject anything else with a usage
error.

#### Registries

The tracer replays two registries in this order.

1. **Committed fixtures** under `tests/corpus/command/tex82` -- today the
   single synthetic `tex82/command-transitions-v1`. Always present, fully
   hermetic, font-independent.
2. **Full-document traces** under `tests/corpus/command/tex82-documents` --
   `plain` (plain-format bootstrap alone), `story`, and `gentle`, each running
   `\input plain` plus the corpus document through real INITEX TeX82. These
   are _generated on demand and gitignored_: one plain trace is about 17 MB
   and Gentle's is about 156 MB, so committing them would add roughly 190 MB
   of exactly-reproducible generated bytes to the repository. A document
   whose trace tree is absent prints a one-line skip notice on stderr and is
   not a failure, so the tracer stays usable on a checkout that has never
   built the oracle.

Both registries produce the same ordered worklist entries; document
divergences follow committed-fixture divergences in the report.

The full-document registry is a manual parity diagnostic only, not a native
test-suite gate. Run the CLI above after generating document traces when
investigating a document-level divergence; a fresh checkout reports absent
generated traces as `PARTIAL` instead of silently treating them as clean.

Once the document tier is present, run the tracer through the `test` profile
(`opt-level = 1`) rather than the plain `dev` profile -- replaying hundreds of
thousands of document events unoptimized takes minutes where the `test`
profile takes seconds:

```bash
cargo run-dev -p tex-command-stream -- --repository . --max-divergences 100000
```

#### Generating the full-document traces

```bash
scripts/build-tex82-oracle.sh --offline          # once, builds the pinned oracle
scripts/build-tex82-document-traces.sh           # all three documents
scripts/build-tex82-document-traces.sh --document plain   # one document
```

`scripts/build-tex82-document-traces.sh` stages
`tests/tex82-documents/<name>/root.tex`, `third_party/corpus/{plain,<name>}.tex`,
`third_party/hyphen/hyphen.tex`, and every `third_party/fonts/*.tfm` into a
fresh run directory; runs the clean and instrumented oracle executables (plus
a third repeat run) and requires their ordinary terminal/log/status/DVI
channels to agree and the instrumented trace to be bit-identical across
repeats; then derives a fully validated `tex-oracle` fixture through
`tex-oracle-bootstrap` and publishes it, with the staged TFMs beside it under
`fonts/`. It performs no network I/O. A partial (`--document`) run prints its
records instead of rewriting the contract, so the pinned file can never
describe a half-regenerated tree.

No run that generates zero traces exits `0`. On a fresh checkout the pinned
oracle or the external inputs are absent, and that is expected rather than
broken -- but it means the tracer's worklist will be short by three documents,
so it gets its own status instead of being folded into either success or
failure:

- `0`: every selected document was regenerated (and, for a full run, the
  contract was rewritten). The final line names how many.
- `1`: generation ran and failed -- an oracle run, a determinism comparison,
  or a fixture bootstrap did not hold.
- `2`: the command line is wrong.
- `3`: a prerequisite is absent, so nothing ran. Run
  `scripts/build-tex82-oracle.sh` and `scripts/setup-conformance-tests.sh`,
  then rerun. Separated from `1` so a caller can tell "not set up yet" from
  "set up, and broken".

Identity is pinned in the committed
`tests/tex82-document-trace-manifest.txt`:

```text
document NAME ROOT-SOURCE FIXTURE-MANIFEST-SHA256 EVENTS FONT-SET-SHA256
```

The fixture manifest digest transitively pins every source, output, and event
byte (`CommittedFixture::load` verifies each file against it), the event count
pins the trace's scale, and the font-set digest -- a SHA-256 over one
`name sha256` line per staged TFM in bytewise name order -- pins the exact
metrics canonical replay registers. A present trace tree that disagrees with
any of these fails the run loudly rather than silently becoming the new
expectation. The same font-set digest is also the fixture manifest's
`distribution_sha256`, because the staged metric set _is_ this tier's
distribution.

Because `CanonicalMainControl::resolve_font_resource` returns
`ExecError::MissingCanonicalFont` immediately instead of suspending, the
tracer registers the whole staged TFM set through
`CommandHostCapabilities::register_font` before the first replay step rather
than through a lazy resource-host retry loop. Replay is bounded by
`(registered input bytes + expected event count) * 2 + 64` deliveries;
exceeding that bound is a contained worklist entry, not an aborted run,
because a document that expands far more commands than it has source bytes
must still terminate under a defect.

### First-Failure Locator

`crates/umber/examples/first_failure_locator.rs` is a standalone diagnostic
entry point for the `umber2-johp` command-core migration, separate from the
DVI-parity Cargo tests above and from the `umber2-johp.28` production
migration itself. Use it for the live end-to-end front, when the differential
tracer's fixture registry does not cover the failing input -- for example, it
depends on live document/font/hyphenation material outside
`tests/corpus/command`.

It stages `third_party/corpus/{plain,<source>}.tex`,
`third_party/hyphen/hyphen.tex`, and the plain-format CM/`manfnt` TFMs into an
in-memory `World` (reusing the same `parity_harness::CORPUS_TFMS`/`locate_tfm`
resolution as the harness above), then drives them directly through
`umber::CanonicalEngineSession` -- no `tex-lex::InputStack`, no legacy
`Executor` -- so it exercises exactly the same canonical command-core path the
migration is converging:

```bash
cargo run --profile test -p umber --example first_failure_locator -- gentle
cargo run --profile test -p umber --example first_failure_locator -- story
```

Use `--profile test` (matching `cargo run-dev`'s alias) rather than the plain
`dev` profile: Gentle and Story are large documents, and an unoptimized
`opt-level = 0` debug build of the still-unmigrated canonical path can take
several minutes where the `test` profile's `opt-level = 1` finishes in
seconds.

It reports the first failure it hits: the live execution mode, the
`ExecError`/`CanonicalSessionError` rendered with provenance-resolved TeX
source context (`ExecError::format_with_provenance`), or, for a Rust panic,
lets the default panic hook report the Rust-side `file:line` origin (rerun
with `RUST_BACKTRACE=1` for a full backtrace). As a first-failure locator (see
the Glossary in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#glossary)),
it can only show that
execution stopped, never that completed output is wrong. It intentionally
does not run under `cargo test`: the command core is mid-migration and this
locator is expected to fail on `gentle` until each earlier divergence in the
`umber2-johp` chain is fixed. `story` currently completes cleanly and is a
regression gate (see "Canonical Story Regression Gate" above): a new `story`
failure is a regression to fix immediately, not the divergence under
investigation. See the current open successor issue under the `umber2-johp`
epic (`bd show umber2-johp` for its children) for the earliest tracked Gentle
divergence it reproduces -- that issue ID advances every time a divergence is
fixed, so it is not pinned here -- and `docs/tex_command_core.md` for the
canonical command-core architecture it exercises.

## TRIP Corpus

The original Knuth TeX82 TRIP and e-TeX V2 e-TRIP workloads are end-to-end DVI
conformance tests governed by the
[End-to-End Conformance Gate Contract](#end-to-end-conformance-gate-contract):
they run when their local inputs and oracles are present and fail with an
actionable report when they are not.

```bash
scripts/fetch-conformance-inputs.sh
scripts/fetch-conformance-inputs.sh --offline
cargo test -p umber --test it e2e_conformance_trip -- --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
```

`scripts/fetch-conformance-inputs.sh` acquires the shared hyphenation and font
inputs, reads `tests/trip-manifest.txt`, tries each entry's locators in declared
order, fetches exact official TRIP and e-TRIP bytes into gitignored
`third_party/trip/`, and verifies every candidate against the entry SHA-256
before acceptance. The tests
use the pinned canonical `trip.tfm`, then run the documented INITEX and
format-loaded TRIP phases in process.

Cargo conformance tests do not launch Umber as a subprocess. Story and Gentle
call the engine directly through the staged fixture callback; TRIP and e-TRIP
share one in-process two-phase format helper.
`scripts/check-and-test.sh` preflights the gitignored e2e oracles before
starting the workspace gate and warns that absent ones will fail their gates.

The Umber integration test gates only the final DVI. Generated logs, terminal
photo, and `tripos.tex` remain diagnostic outputs in the separate diagnostic
parity tier. Its oracle normalizes only the DVI preamble comment and otherwise
requires byte identity with the committed, locally pdfTeX-generated fixture.
Regeneration executes the two-phase workload from `trip.tex` and `trip.tfm`
and never copies the official `third_party/trip/trip.dvi`.

DVItype remains diagnostic. Failures write byte, page, opcode, and
disassembly context under `target/conformance-triage/trip/`. See
[TRIP](trip.md) for the exact source pins and normalization policy.

## Specialized Guards

`tex-out` owns the cross-crate page-output float guard. Its unit tests scan the
page node, packing, shipout lowering, artifact, DVI, and CLI DVI composition
sources and fail if float types or float rounding APIs enter that fixed-point
path. Its allowlist is limited to documented non-arithmetic fixture or
formatting false positives.

The explicit LaTeX tier is split by boundary. `scripts/check-latex-corpus.sh`
builds the pinned native format, runs the four base classes for three passes,
compares DVI and auxiliary artifacts with TeX Live 2026, and verifies the
30-input `tests/latex-runtime.lock` closure. This seed fixture is not the
production distribution: `scripts/build-texlive-snapshot.sh` enforces full
runtime inventory floors and package metadata hints. `scripts/check-latex-wasm.sh`
publishes that closure with the format, builds the real WASM package, and
exports that same format explicitly for the native run before requiring
byte-identical three-pass native/WASM article parity. Neither command belongs
in the ordinary workspace test tier because both intentionally build live
pinned distribution artifacts.

`scripts/test-publish-texlive-r2.sh` is the hermetic contract test for the R2
release command and runs in `scripts/check-and-test.sh`. Mock rclone and curl
boundaries cover dry-run behavior, failure followed by resumable rerun,
credential non-disclosure, bounded transfer/checker/retry flags, non-deleting
immutable copies, exact remote inventory checks, manifest-last ordering, and
public digest/CORS verification. It performs no network requests and uploads
nothing. The production staging and public origin are verified only by an
explicit coordinator invocation of `scripts/publish-texlive-r2.sh`.

The upstream LaTeX2e DVI tier is also explicit:

```bash
scripts/setup-latex-parity-tests.sh
scripts/setup-latex-parity-tests.sh --offline
scripts/check-latex-parity.sh --offline
scripts/check-latex-parity.sh --offline --format target/latex-parity/format/latex.fmt
scripts/check-latex-parity.sh --self-test-format-reuse
scripts/check-latex-parity.sh --self-test-reference-lookup
```

`tests/latex-parity-manifest.txt` pins the complete official
`release-2024-11-01-PL2` repository archive by commit, byte length, and SHA-256;
it does not pin individual support or test files. Setup extracts the unmodified
LPPL snapshot under gitignored `third_party/latex2e-parity/`, then derives every
same-stem standard-`.tlg` shipout candidate under `base`, `required/tools`,
`required/graphics`, and `required/amsmath`. The pinned tree yields 295
candidates. A live classic-LaTeX census emits DVIs for 286 of them and records
the nine exact manifest-pinned alternate-configuration paths separately;
unexpected reference DVI absence or presence fails the tier. The manifest retains
`base/testfiles/sx172785.lvt` in that 286-case reference-DVI cohort but skips it
explicitly as `unsupported-pdftex-primitives:pdfprotrudechars,rpcode`; this is
the only unsupported case, leaving 285 applicable classic-DVI comparisons.
Offline mode rejects a missing or changed archive cache without accessing the
network.

Without `--format`, the checker invokes the verified format builder exactly
once before entering the case loop. With `--format`, it invokes the builder
zero times. It hashes that pregenerated image, copies those exact bytes into a
fresh directory for every applicable reference/Umber pair, and each of the 285
Umber DVI runs loads the local copy with `--format latex.fmt`. The unsupported
case does not start an Umber run, so a complete current tier restores the
format exactly 285 times. The persistent
`target/latex-parity/last-run-format-receipt.txt` records the builder count,
source identity, and all 285 per-case identities; the fast self-test asserts
one build and three identical restores. A separate fast lookup self-test
accepts the declared snapshot, distribution, per-case, generated-state,
configuration, and format inputs while rejecting both a direct ambient input
and a symlink escape.

Each reference invocation starts from an empty environment with only the host
executable path and its deterministic clock/locale plus explicit kpathsea
settings. `TEXINPUTS` and `TEXFONTS` have no default-search suffix;
`TEXMFHOME`, `TEXMFCONFIG`, `TEXMFVAR`, caches, temporary files, and generated
fonts all point beneath that case's scratch root. The distribution's one
prebuilt `latex.fmt` remains an exact allowed file, not a general allowance for
ambient `texmf-var`. After every reference invocation, including non-DVI
configurations, the `.fls` recorder paths are canonicalized and must belong to
the case directory, pinned upstream snapshot, `texmf-dist`, isolated generated
state, or the two exact distribution configuration/format files. This check
runs before recorder-discovered input or TFM directories are passed to Umber.

The runner continues after individual
engine or DVI failures and writes complete persistent census lists to
`target/latex-parity/last-run-failures.txt` and
`target/latex-parity/last-run-non-dvi.txt`; explicit exclusions are recorded
separately in `target/latex-parity/last-run-skipped.txt`. The full-cohort
accounting requires tested plus skipped classic-DVI cases to equal the
manifest's 286-case reference-DVI count. Unless `--keep-work` is explicit, each
isolated reference/Umber pair is removed as soon as its result and compact
triage artifacts have been recorded, and the scratch root is removed on both
success and failure. This bounds temporary format-copy storage to one active
case instead of retaining all 285 copies after an expected census failure.
Reference and Umber cases have a 60-second timeout so one recovery loop cannot
stall the census without misclassifying the slower tools cases under
full-corpus load; set `UMBER_LATEX_CASE_TIMEOUT_SECONDS` to tune that explicit
tier locally.

Acceptance ignores transcript and process-status differences when an
intentional diagnostic still leaves a DVI. It removes stale DVI before every
pass and requires a newly emitted file, then normalizes only the existing DVI
preamble comment and otherwise requires byte identity. Mismatches write raw
DVI, first-byte context, page-limited disassemblies, and the divergent page and
opcode under `target/latex-parity/triage/<case>/`. This live TeX Live tier and
its roughly 74 MB format build remain outside ordinary Cargo tests.
