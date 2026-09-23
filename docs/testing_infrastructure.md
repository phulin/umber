# Testing Infrastructure

Status: current repository reference
Scope: the test commands, limits, fixtures, corpora, and harnesses
that exist in this workspace today.

This document records current implementation facts: what each tool is and how
to run it. For rules that should guide future test design and placement, see
[Rust Testing Policy](testing_policy.md). For the _process_ of working a
`umber2-johp` canonical/oracle divergence with these tools -- diagnosis order,
oracle hierarchy, fix discipline, gates, and the glossary defining that
vocabulary -- see [Canonical Divergence Working Contract](canonical_divergence_workflow.md).
This document does not restate that process.

---

## What the tests protect

These classes describe the contract a test checks. They are independent of the
command that runs it and of Rust's unit/integration visibility boundary. One
case may exercise several classes; name its primary owner and keep distinct
assertions when the same input supports different contracts.

| Class                   | What it checks                                                                    | Why it exists                                                     |
| ----------------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Local rules             | Arithmetic, scanning, sorting, and other focused algorithms.                      | Small boundary cases locate a defect quickly.                     |
| Reference compatibility | Declared TeX, e-TeX, pdfTeX, BibTeX, or Biber outputs against pinned authorities. | Agreement with Umber's own output cannot establish compatibility. |
| State and lifecycle     | Retry, rollback, edit, cancellation, and effect publication.                      | Transitions can fail even when local rules are correct.           |
| Formats and outputs     | Format images, detached artifacts, DVI, PDF, and HTML.                            | A successful run can still publish invalid output.                |
| Product and platform    | CLI, native sessions, WASM bindings, browser worker, and package.                 | Library correctness does not prove caller integration.            |
| Limits and performance  | Bounded hostile inputs, reclamation, allocation, and scaling.                     | Equality alone does not detect hangs or unacceptable cost.        |
| Test and tool integrity | Discovery, fixture authority, gate selection, and failure reporting.              | A broken harness can report false success.                        |

## Commands, prerequisites, and results

Run a focused crate while iterating; run the native suite and quality gate for
routine verification. The optional checks are selected when their subsystem is
changed. A command's result covers only its selected tests and declared
prerequisites.

| Lane            | Command                                                             | Prerequisites                                            | Result and scope                                                                                                              |
| --------------- | ------------------------------------------------------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Focused         | `cargo test -q --tests -p <crate> [filter]`                         | Rust toolchain and that crate's fixture assets           | Selected tests only.                                                                                                          |
| Routine native  | `cargo test -q --tests`                                             | Rust and the pinned local assets; no live TeX executable | Every host-testable default member and committed fixture gate.                                                                |
| Routine quality | `scripts/check.sh`                                                  | Rust, dprint, Node/Biome                                 | Formatting and clippy feature resolutions; no Rust tests. Named gates select only those gates.                                |
| Combined        | `scripts/check-and-test.sh`                                         | Native and quality prerequisites                         | Prebuild, guarded native tests, quality, publication/asset/gate contracts; aggregate `PASS`, `FAIL`, `BLOCKED`, or `PARTIAL`. |
| Subsystem       | `scripts/check-wasm.sh`, `scripts/check-tools.sh`, LaTeX/PDF checks | Tools and assets required by each named step             | Explicit selected steps; absence is `BLOCKED` (exit 4), subset is `PARTIAL`.                                                  |
| Extended        | Named corpus, fuzz, snapshot, and performance commands              | Pinned datasets and toolchain for that command           | Its named workload only, never implied by the routine gate.                                                                   |

The native gate uses Cargo `default-members`. The
`default_members_cover_every_host_testable_crate` and
`every_excluded_workspace_directory_names_its_check` tests in
`crates/test-support/tests/workspace_selection.rs` enforce selection against
Cargo metadata. The sole omitted root member is `umber-wasm`, whose
`#[wasm_bindgen_test]` bodies do not run on a host target. The two excluded
workspaces are `tools/fixturegen` and `tools/texlive-wasm-publish`; their tests
run through `scripts/check-tools.sh`. The same selection suite rejects dormant
`#[cfg(any())]` production code and test modules hidden under disabled library
targets. These guards prove discovery and source authority, not individual
behavioral coverage.

`scripts/check-and-test.sh` builds the native suite before running its test
binaries under a 30-minute/6-GiB process-group guard concurrently with
`scripts/check.sh`. Those limits prevent runaway work; they are not a warmed
feedback-time budget. No portable time threshold is claimed here. Measure
cold compile/link, warmed execution, and quality-gate time separately on a
quiet comparable host.

A `PASS` means every selected step ran and met its contract. `FAIL` means an
executed check failed. `BLOCKED` means a required prerequisite was absent and
the check did not run (exit 4 in optional and aggregate scripts). `PARTIAL`
means a deliberately selected subset or an explicit coverage reduction, not a
complete pass. `UMBER_CONFORMANCE_ORACLES=optional` lets missing local DVI
oracles reduce coverage; the combined script reports `PARTIAL` and the bare
Cargo command must be described with that reduction. By default missing
required oracles fail their Rust gates. See the
[End-to-End Conformance Gate Contract](#end-to-end-conformance-gate-contract).

Within compatibility suites, distinguish these case statuses too:

| Case status            | Meaning                                                                                                                                            |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Matched                | The case executed and all selected channels matched.                                                                                               |
| Executed known failure | The case executed, reached its reviewed divergence, and matched a strict fingerprint. A changed failure or unexpected pass is a failure to review. |
| Ignored                | Rust did not execute the case. An ignored compatibility declaration is inventory, not a verified known failure.                                    |
| Blocked                | A prerequisite prevented execution; no result about behavior exists.                                                                               |
| Unselected             | The command did not include the case. It contributes no result to this run.                                                                        |

The pinned bibliography compatibility tree currently contains ignored
upstream cases. Its declaration audit checks their metadata; it does not turn
those cases into executed expected failures. This documentation change does
not alter bibliography selection or status.

`scripts/script-suite-inventory.tsv` indexes executable script suites by
class, lane, aggregate owner, and prerequisites.
`scripts/test-script-suite-inventory.py` compares the inventory to discovered
scripts; `scripts/test-gate-verdicts.sh` is its routine owner. This inventory
also names standalone manual commands without claiming the aggregate runs
them. `scripts/check-tools.sh`'s `oracle-contract` step validates the pinned
regeneration contract through `scripts/test-oracle-regeneration.sh`; live oracle
construction remains an explicit `scripts/regen-fixtures.sh` maintenance
operation.

Routine tests read committed fixtures and provisioned local oracles without
invoking reference TeX. Provision the primary checkout once with
`python3 scripts/provision.py worktree .`; provision each linked checkout with
`python3 scripts/provision.py worktree <worktree>`. Regenerate committed
reference fixtures only through `scripts/regen-fixtures.sh`, then inspect the
changed evidence. The TeX82 property-catalogue gate in `test-support` checks
the committed 1,380-module inventory, typed deferrals, citations, ownership,
and exact active Rust test links. Its catalogue labels describe those claims,
not a compatibility percentage.

Bounded execution, lexical, and session cases under `tests/corpus` use closed
Git directories: each case owns its source, local support files, and declared
`expected.<channel>` outputs. The shared `ClosedCase` validator rejects missing,
extra, ignored, untracked, symlinked, and nonlocal authorities. Live reference
generation for `exec` and `typeset` writes pdfTeX INITEX terminal output,
matching the CLI channel under test; it does not substitute the different log
channel. `etex_exec` retains its extended INITEX log contract. The generators
stage the same printable catcodes as a fresh Umber job and use committed TFM
metrics. `tex_exec`'s historical `expected.ref` files have no pinned capture
generator, so that regeneration area validates them without rewriting them.
The regenerated command-semantic cases retain strict per-channel xfail
fingerprints. [Fixture Regeneration](#fixture-regeneration) gives the commands
and oracle identities.

### Optional checks

| Entry point                          | Scope                                                                                                                                     |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `scripts/check-tools.sh`             | Excluded host-tool workspaces, `parity-harness` with `reference-tools`, profiling targets, tool contracts, and opt-in clippy resolutions. |
| `scripts/check-wasm.sh`              | WASM target, binding tests, authored JavaScript, and generated browser package checks.                                                    |
| `scripts/check-hb-shape-fixtures.sh` | Rustybuzz against C HarfBuzz.                                                                                                             |
| `scripts/check-latex-corpus.sh`      | Pinned native LaTeX corpus and runtime closure.                                                                                           |
| `scripts/check-latex-wasm.sh`        | Native/WASM LaTeX article parity.                                                                                                         |
| `scripts/check-latex-parity.sh`      | Upstream LaTeX2e DVI parity cohort.                                                                                                       |

Optional runner verdicts list selected, passed, failed, and blocked steps.
Naming a step runs exactly that command and reports `PARTIAL` for the whole
subsystem. Consult the entry point for its precise current step list and
required programs. In particular, browser package integration requires the
generated WASM package and an actual browser; a Node mock or an unavailable
placeholder cannot certify a worker round trip. Report the final worker result
only after that step executes.

### Declarative Command Semantic Minifixtures

Run the fast property-scoped semantic tier independently with:

```bash
cargo test -q -p tex-command-stream --test it command_semantic
```

Schema, inventory, route, and bounded command-behavior checks remain in that
routine selection. The two full exact-comparison checks are temporarily a
manual parity tier while `umber2-alfh.11` owns the terminal-EOF divergence:

```bash
cargo test -q -p tex-command-stream --test it command_semantic -- --ignored
```

That command is expected to fail until the tracked semantic defect is fixed;
it preserves the exact fixture path without making known parity work a routine
cutover gate.

The Umber integration binary applies the same classification to currently
failing transcript/DVI corpus, pdfLaTeX compatibility, Gentle, and focused
loaded-TRIP assertions during the command-core cutover. Their bodies and
assets remain available through the explicit manual path:

```bash
cargo test -q -p umber --test it -- --ignored
```

Each such test carries an `ignore` reason naming the manual
compatibility/parity tier. Passing command-only smoke, restart, replay,
serialization, resource, and end-to-end assertions remain routine.

The one Cargo integration binary discovers independent fixture directories under
`tests/corpus/command-semantic/<domain>/<fixture>/`; adding a domain or fixture
does not edit a shared Rust registry or add a top-level integration target. Each
fixture directory is a closed unit containing exactly one versioned,
singleton `manifest.json`, its declared TeX source, and every applicable
committed channel as `expected.<channel>`. Domain directories contain neither
case manifests nor shared expected-output trees. The local manifest binds the
tiny source to its catalogue property, exact canonical authority and sections,
projection kind, short expected observations, and either `pass` or a strict
`xfail` expectation. Discovery rejects malformed, duplicate, unsafe, or unowned
cases and sources, nonlocal or untracked files, symlinks, and channel files
outside their owning fixture directory. A manifest whose short directory name
differs from its catalogue shard declares `property_domain`; ownership
validation remains exact.

The corpus contract -- manifest parsing and validation, the bounded canonical
run, and the projections -- lives in `tex_command_stream::semantic`, not in the
test binary, so the regeneration path drives exactly the code the gate does.
The test binary holds only the assertions.

The runner drives each input through instrumented
`tex_exec::MainControl` in the exact TeX82 INITEX profile. Each case has two
explicit completion projections over the same canonical driver, profile,
source, and host inputs. Its semantic projection, event count, and status stop
at the authored-fragment root EOF. Its terminal, log, DVI, and effects channels
continue as a real TeX job through TeX82 §360, because the reference pdfTeX
process exposes no host-fragment completion boundary. The runner compares the
two executions through their typed termination observation and rejects any
earlier divergence, so the split cannot mask driver or state drift. The
complete-job projection is framed with
`MainControl::begin_job`/`finish_job` exactly the way
`docs/job_framing.md` describes -- the start-up banner, the `**` line, the
root source registered by name so §537/§362 bracket it in `(name`/`)`, and
§642's page report and transcript line once the run ends -- and it runs in
`Case::interaction_mode`'s engine mode, which defaults to
`InteractionMode::Scroll`, matching the oracle runner's own default
`-interaction=scrollmode` (see that script's "Interaction mode" comment for
why: scrollmode is the one mode that both tolerates the `\read`/`\pausing`
cases this corpus feeds terminal answers to and omits the error-stop prompt
an undeclared divergence would otherwise demand an answer for). A case that
needs a different mode declares one explicitly, together with a nonempty
`interaction_mode_note` explaining why its channels are not comparable to the
standard scrollmode sweep (`validate_case` requires the note whenever the mode
is non-default, and requires its absence otherwise); the oracle runner reads
the same declared mode per case, so the two sides stay comparable even away
from the default. `main-control/show-completion` is the one committed case
that does this: it exists to exercise the `?␣` prompt only `errorstopmode`
issues after `\showthe` (tex.web §1298), which no scrollmode run could ever
produce. The two profiles built past INITEX (`etex-loaded`, `production`)
cannot take `begin_job`'s banner at all, and the oracle runner cannot
reproduce either profile, so their 5 cases run unframed, exactly as every case
did before this framing existed. The runner compares the declared concise
projection of committed command observations or selected
canonical-main-control boundaries -- mode changes, final box-register node
outlines, and committed shipout artifact identities -- and, separately, the
per-channel contract described below. An xfail must link a concrete Beads bug
and pin the first mismatch's index, kind, expected value, and actual value;
XPASS and changed-failure results fail the test. Nothing uses `#[ignore]`,
`should_panic`, a live TeX process, a format or fonts, or the generated
long-document trace registry.

The concise `terminal-checks` projection searches only TeX82 §54's
terminal-visible `term_only` and `term_and_log` sinks. A `log_only` write is
excluded from that projection and remains available through the independent
log channel, so interaction-selector evidence retains its exact routing.

The corpus holds 207 fixtures across 9 domains. Bounded in-memory terminal
lines and named inputs keep the pausing, read, and input-open evidence
hermetic.

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
coverage of the run, and for a long time it was standing in for one. Before
per-channel coverage was introduced, measured corpus runs produced far more
observations than their concise projections declared, including shipped pages
and complete log streams that no projection compared. **A projection is an
omission with a schema**, which is the same defect as `default-members` naming
21 of 34 crates: an absence that reads as coverage.

So each case declares a `channels` block accounting for every observable its
run produces, and the gate compares all of them alongside the projection:

- `events`, the exact committed-observation count. Counted rather than
  committed, because the canonical event stream's oracle-backed home is the
  `tests/corpus/command/tex82` fixture tree and duplicating it here would
  commit an Umber self-golden;
- `status`, either `clean` or `fatal:<label>` for a §81 `jump_out`;
- `terminal`, `log`, `dvi`, and `effects`, each `empty`, `file`, `xfail`, or
  `xfail-diagnostics`; `effects` alone may instead be `unsupported` with a
  reviewed nonempty reason and no expected bytes. A fixture-local
  `expected.<channel>` file is required for `file` and both xfail forms, and it
  always holds the pinned reference engine's bytes (see below).
  The corpus commits applicable terminal, log, DVI, and effects files.
  Terminal and log both grew from a minority of cases to nearly every one once
  job framing gave every run a banner, a `**` line, and a page report or
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
bug instead of becoming the escape hatch. No case holds it today: the three
that used to -- `input-expansion/expansion-conversions`,
`input-expansion/input-start-file`, and `main-control/read-to-definition` --
all reach the end of their run,
and `only_unrunnable_xfail_cases_are_exempt_from_the_channel_contract` keeps
the set empty by re-running every candidate rather than by anyone remembering.

**Every committed fixture-local `expected.<channel>` file holds the pinned
reference engine's bytes -- that is the one meaning a committed channel file
has, for `file` and `xfail` alike (`umber2-alfh.7`).** `StreamDisposition`
therefore carries no `authority` field: there is exactly one place a
committed channel's bytes can have come from, so a field that distinguished
where they came from would carry no information. That was not always true.
Every committed channel file used to record an `authority`, and until
`umber2-alfh.1` all 274 of them held an unadjudicated implementation-observed
origin: this implementation's own observed output, pinned against silent
drift but not yet checked against anything. An `xfail` channel's committed
file held Umber's own known-wrong bytes and the comparison was byte-identity
against that self-pin -- indistinguishable from `file` except in name.
`umber2-alfh.1` promoted every channel to the pinned instrumented pdfTeX
1.40.29 oracle (`scripts/run-minifixture-oracle.sh --all`, which also builds
the two profiles built past INITEX -- `etex-loaded` and `production` -- from
a real `\dump`/`-fmt` roundtrip rather than skipping them) and deleted that
unadjudicated origin from the Rust type, the JSON schema, and every manifest,
so an unadjudicated channel can no longer be recorded at all.

An `xfail` channel carries a `mismatch`: the first line at which Umber's own
output diverges from the committed reference, both sides rendered so a
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

`xfail` writes a whole channel off: nothing after the pinned line is compared
at all, and every improvement to a diagnostic moves the pin and has to be
absorbed by a regeneration. `xfail-diagnostics` is the narrower disposition
for the common case where the divergence _is_ the diagnostic. It names a
`bug`, pins no line, and keeps comparing the channel with tex.web §82's error
reports cut out of both sides, so the file framing, page output, and job tail
a divergent report used to hide stay under test. `strip_diagnostic_reports`
does the cutting, and it recognizes a report without knowing which error
raised it, because §82 frames every one the same way: §306's
`Runaway <status>?` heading and its one line of partial token list, then
`print_err`'s `!␣` line, then `show_context`'s levels and §90's help lines up
to the first empty line -- which can only be `error`'s own closing
`print_ln`, since a context level's second line is padding spaces and no help
line is empty. §83's `error_stop_mode` arm is deliberately not modelled: it
returns from `error` at `prompt_input("? ")` having printed neither help nor
that blank line, and on the terminal `term_input`'s `term_offset:=0` puts the
next output on the same physical line as the `?`, so there is no line-level
boundary to cut on. A channel whose reports end that way stays `xfail` --
`main-control/empty-token-register` is the one that does. A divergence that
escapes the reports fails as `DiagnosticsAside`, naming the bug alongside the
line that escaped; matching the reference raw is an xpass, exactly as it is
for `xfail`. Only `terminal` and `log` may declare it, because `dvi` and
`effects` carry no §82 reports and the disposition would silently mean
"compare normally" there.

Two byte ranges carry a wall clock that no two runs of the same job can ever
agree on, and both are normalized -- by one function,
`tex_command_stream::semantic::normalize_channel`, which the ongoing gate and
the regeneration tool share so that a channel written as `file` cannot fail
under the gate that reads it back:

- The log channel's clock (tex.web §536, on the log's first line only),
  through `normalize_log_clock` (`docs/job_framing.md`).
- The `dvi` channel's preamble comment (tex.web §617's `pre` comment), through
  `test_support::dvi::normalized_dvi_for_comparison` -- the same normalization
  the byte-exact DVI parity harness has always applied, which rewrites exactly
  the declared `k`-byte comment payload and requires every other byte,
  including `k` itself, to already match.

Both are idempotent, and both are applied to the committed reference and to
Umber's freshly captured bytes alike before any comparison, so a committed
file is stable across regenerations regardless of which day the oracle was
captured. Nothing else is normalized away.

The effects channel is a deterministic JSON Lines projection of the shared
`tex_oracle::EffectEvent` schema. It retains only reference-observable numbered
stream `open`, `write`, and `close` events, in event order, followed by exact
generated-file artifacts in bytewise logical-path order. Terminal/log writes,
shipout, termination, and specials are omitted because the terminal/log, DVI,
status, and DVI channels own those observations. Each artifact record carries
its logical TeX output name and exact bytes; host paths and Umber-internal
effect records are never serialized. Regeneration derives the event records
only from the pinned oracle observation stream and reads the declared oracle
artifacts, so it cannot bless an Umber self-baseline. `unsupported` records an
explicit absence of a portable verdict; regeneration preserves that review
decision and cannot manufacture expected bytes for it.

Complete-job capture commits the final staged effect suffix before reading
those artifacts. This is the ordinary driver finalization boundary for TeX82
§§1373--1375, and it matters even when no page shipped: an immediate
`\openout` followed by `\closeout` still creates an exact empty file. The
authored-fragment run retains its suffix without materializing host output, so
the root-EOF rollback contract remains independent of complete-job effects.

The `dvi` entry was added late (`umber2-alfh.22`). Until then this corpus
compared the preamble comment raw while the rest of the repository held it
uncomparable, which pinned 66 cases as `xfail` for differing only in a
banner. Masking it left exactly one real DVI divergence in the corpus
(`umber2-86sl`, a `\special` written ahead of its box's glyphs), which had
been invisible because the channel fingerprint records only the _first_
divergence and the banner always came first. It is fixed; the point stands
that only normalizing the banner made it visible at all.

The effects projection makes stream ordering and generated artifacts
reference-adjudicable. The three focused TeX82 cases cover open/close without
a write, a top-level open/write/close sequence, and the stream-selector
boundaries at `\closeout`; exact mismatches remain strict xfails linked to
their implementation bugs. The pre-existing divergent terminal and log
channels remain linked to their own bugs:

- `umber2-alfh.25` (a file's `)` is closed early): 4
- `umber2-alfh.26` (Umber raises a _different_ error than pdfTeX): 4
- `umber2-alfh.11` (the `*` prompt / terminal-read residual): 6

Read those counts as channels, not as defects. `terminal` and `log` are not
independent evidence: TeX writes most of a job's transcript to both at once
(§54's `term_and_log`), so one divergence normally pins two channels, and the
14 above are 7 distinct case-level divergences. Every `dvi` channel now
matches, which is what the entry above was added to make measurable.

Regenerate the contract with:

```bash
scripts/regen-fixtures.sh --area command-semantic
```

It drives the same `tex_command_stream::semantic` module the gate does, so a
regenerated contract cannot describe a run the gate would not reproduce. It
consumes the separately guarded pinned-oracle capture and writes only the
owning fixture directory. The emitted block matches `dprint`'s own shape and
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
`scripts/check-tools.sh` lints through `umber`. Run that explicit check when
changing those opt-in configurations.

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

Native `umber run` commands expose the independent per-revision guards as
`--expansion-fuel` and `--execution-steps`. Explicit flags take precedence over
the compatibility environment variables `UMBER_ENGINE_FUEL` and
`UMBER_ENGINE_STEPS`; without either form, the ordinary execution-step cap
remains exactly 10,000,000. An explicit guarded run prints one `RUN_GUARDS`
diagnostic naming `expansion_fuel_cap` and `execution_steps_cap` separately.

The pinned 50M pdfLaTeX authority command is
`scripts/run-pinned-pdflatex-50m-authority-row.sh`. It fixes expansion fuel at
50,000,000 and committed executor steps at 100,000,000, the validated hard
maximum. The latter is 10 times the independently observed ordinary-step
endpoint and is headroom, not a conversion between step and fuel units. Pass
the same script and inputs to both binaries in a matched comparison. Its
`authority.receipt` records both caps, their distinct units, the distribution
pin, source epoch, prefetch count, and the SHA-256 identities of the binary,
input, format, distribution root, and ordered prefetch closure. Invoke it as:

```bash
scripts/run-pinned-pdflatex-50m-authority-row.sh \
  BINARY SOURCE_ROOT INPUT FORMAT DISTRIBUTION DISTRIBUTION_AHASH64 \
  PREFETCH_KEYS OUTPUT SOURCE_DATE_EPOCH
```

The explicit stepwise recent-arXiv validation tier is:

```bash
cargo build -q --profile test -p umber --bin umber
UMBER_ARXIV_FORMAT=/path/to/pdflatex.umberfmt \
UMBER_ARXIV_DISTRIBUTION=/path/to/verified/texlive-snapshot \
  scripts/run-stepwise-arxiv-census.sh
```

The clean-pdfTeX PDF-success denominator is a separate reference-only tier
owned by `scripts/survey-pdftex-arxiv-pdf.py`. It selects the rows whose
archive `00README.json` declares `pdflatex`, materializes each complete locked
archive into an issue-local row directory, preserves the source-derived
`\jobname` by never passing `--jobname`, and runs from the archive root with
normal side files. The command requires explicit clean-oracle, oracle-build,
paired-format, format-receipt, TeX Live runtime, archive, and output paths.
After the survey, pass the same arguments with `--verify-only`; that path
launches no compiler and rehashes the complete source and artifact evidence
before reproducing the ordered report and totals. The 2026-09-02 capture is
`target/arxiv_census/recent-20260902-pdftex-pdf/`: 87 of 94 declared-pdfLaTeX
rows produced authoritative reference PDFs, six stopped at an undefined
control sequence, and one hit the 120-second guard with a non-authoritative
partial PDF. This tier must not invoke Umber, inspect an Umber PDF, or patch a
paper.

The canonical per-document parity workflow is separate from the census. A
candidate qualifies only when its complete, unmodified archive compiles
cleanly with the pinned TeX Live 2026 pdfTeX in both DVI and PDF modes; retain
the two output identities and page counts. PDF compilation is only an
eligibility check in this pass, and PDF-only or otherwise non-DVI-capable rows
are recorded for the later PDF pass. Then compile the complete source once
with Umber in DVI mode under 500,000,000 expansion fuel, the ordinary
10,000,000 execution-step cap, and the established wall-time, RSS, and
termination-grace guards. Fuel is solely a nontermination guard and is never a
parity metric. Compare DVI through `parity-harness --compare-existing-dvi` and
stop at the first meaningful semantic or DVI divergence. After complete DVI
parity, advance directly to the next eligible source in the locked corpus. Do
not run, inspect, render, or use Umber PDF output for diagnosis until that
corpus-wide DVI pass is complete. Prefix boundary searches, per-page
recompilation, serialization-only PDF differences, font-subset tags, and
extractor rounding are not parity work.

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
stable clusters. Census children also set `UMBER_CAUSAL_DIAGNOSTIC=1`. A
failed engine compile then emits exactly one `CAUSAL_DIAGNOSTIC` line before
the ordinary terminal summary. The line carries a stable cause family, hashes
of the terminal cause and virtual source path, exact byte/line/column
coordinates, and innermost-first tails of at most eight input frames and eight
groups. It contains no source excerpt, token-list contents, macro arguments,
or unhashed path. The runner rejects repeated or larger-than-1,024-byte lines
and stores the accepted line in the row JSON receipt; successful rows emit no
line and retain `null` in that field.

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

Pure typesetting has an explicit performance tier owned by the standalone
`tex-typeset` benchmark crate:

```bash
cargo bench --manifest-path benchmarks/tex-typeset/Cargo.toml --bench widths
cargo bench --manifest-path benchmarks/tex-typeset/Cargo.toml --bench layout
cargo run --release --manifest-path benchmarks/tex-typeset/Cargo.toml \
  --bin layout_allocations
scripts/check-node-width-budget.sh
```

The width script preserves the committed row names, means, and 10% tolerance.
It validates the baseline schema and exact row set before running Criterion,
then applies those timing limits only when the active Rust host triple and
exact compiler release match the baseline metadata. Other environments report
a machine-readable, non-gating `unsupported` result and exit `4`; they do not
claim either pass or regression. `scripts/check.sh` preserves that status as
`BLOCKED` rather than relabeling it as `PASS` or `FAIL`.
The allocation binary preserves the alignment, line-breaking, deep-choice,
deep-sublist, and flat-math ceilings. The incremental two-generation accepted
and rejected edit diagnostic is separately runnable from its owner with
`cargo bench --manifest-path benchmarks/tex-incr/Cargo.toml --bench accepted_edit`.

Authenticated native distribution startup has a focused hermetic benchmark:

```bash
CARGO_BUILD_JOBS=1 cargo run --release -p umber --bin distribution-startup-benchmark
```

It creates a synthetic one-shard pinned distribution and warms only its owned
temporary cache. The cold route launches five actual child processes; the
same-process route starts five fresh compile sessions under one bounded
`NativeDistributionOwner`. Both routes consume the identical verified cache
inventory. The executable reports manifest reads, parses, authentications,
owner hits, shard loads, object hashes, and cache hits, and fails unless owner
reuse reduces every manifest-work counter. It also requires byte-identical DVI
output and a byte-identical complete cache inventory before and after the
measurement, so the timing row cannot hide output loss, hash bypass, cache
rewrites, network acquisition, or corpus prewarming. Its shard also carries a
valid unrequested file and dependency hint; both routes must report exactly one
object hash per compile, proving live work follows the one requested file.

Complete distribution and cache integrity has a separate explicit verifier:

```bash
CARGO_BUILD_JOBS=1 cargo run-dev -p umber --bin distribution-verify -- \
  --distribution target/texlive-snapshot \
  --distribution-sha256 <pinned-root-sha256> \
  --cache <umber-cache-root>
```

This command is intentionally outside compilation. It walks every authenticated
shard and referenced distribution object and every current cache blob, reports
exact hash counts and bytes, and fails on mutation. Routine hermetic controls
cover successful complete audits plus corrupt root, unrequested object, and
cache payload failures. Native resolver controls separately prove that corrupt
unrequested closure/dependency objects are not opened or hashed by a normal
compile.

Final state and coarse generation ownership have a separate explicit
performance tier:

```bash
scripts/check-snapshot-budgets.sh
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets
```

The script enforces zero warmed allocation for direct reads, operation-local
assignment rollback, and page-queue reuse, plus exact prior/current owner
lifecycle as described in [Final State and Generation Performance](snapshot_performance.md).
The Criterion command reports the same direct state operations and cold coarse
generation construction. Neither belongs in the default cargo-test tier.

Dependency observation has a separate state performance diagnostic:

```bash
cargo run --release --manifest-path benchmarks/tex-state/Cargo.toml \
  --bin dependency_gate
```

It compares disabled recording, active unique reads, unchanged validation, and
semantic backdating while reporting the retained detached observation bytes.

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

Long-session ownership has one fast routine smoke test and one explicit stress
tier:

```bash
cargo test -q -j 1 --tests -p tex-incr \
  tests::long_session::long_session_thousands_plateau_at_equal_work_milestones \
  -- --ignored --exact --test-threads=1
```

After 64 warm-up cycles, the stress tier performs 2,048 accepted editor
patches and 2,048 completed-but-rejected patches. Every rejected patch crosses
one real `NeedResource` suspension and fulfillment before drop; accepted jobs
exercise redefinition, group restoration, glue ownership, shipout, and named
checkpoints. At equal 128-cycle milestones it compares reachable checkpoint
state, effects, artifacts, DVI plans, and DVI bytes with a clean rebuild. Exact
live token, macro, glue, provenance, source, journal, and node owner categories
must remain constant, while weak indexes, checkpoint roots, provenance
storage, and node storage remain within their declared budgets. Fragment
metadata has a 64-row retired-coordinate budget; after warm-up, both
`diagnostic_bytes` and `checkpoint_root_bytes` must equal their baseline
exactly rather than consume patch-count headroom. Equal-work receipts pin
accepted/rejected/retry/checkpoint counts, delivered commands, and fuel
independently of retention.

On Linux the same milestones sample `/proc/self/status` and allow at most 64
MiB of process-RSS growth after warm-up. RSS is allocator/process diagnostic
evidence only: it never establishes reachability, equality, acceptance, or
output authority. Run this tier alone with one test thread after checking the
owning cgroup's `memory.events` and ensuring no heavy peer is active.

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
for DVI fixture updates. Its `--area pdf` mode requires pdfTeX 1.40.29 and
Poppler `pdftoppm` 25.08.0; its `--area fonts` mode owns the explicit live
`tftopl` cross-check and does not rewrite fixtures.

Its `--oracle tex82 --profile initex-eight-bit` and `--oracle etex26
--profile compatibility+extended-eight-bit` modes own pinned live reference
builds outside the correctness tier. Both reuse the hash-verified
TeX Live 2026 source cache offline, record the source/change/tool/platform and
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

The `--oracle pdftex14029 --profile initex-etex-eight-bit` mode performs the
corresponding pinned pdfTeX 1.40.29 build. It gates DVI/PDF smoke artifacts,
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

### Acquiring the pinned TeX Live 2026 source archive

`scripts/build-tex82-oracle.sh` fetches the pinned
`texlive-20250308-source.tar.xz` from the single host recorded in
`tests/trip-reference-manifest.txt`, `ftp.math.utah.edu`. That host fails TLS
verification on some networks:

```text
curl: (60) unable to get local issuer certificate
```

The same failure has been observed on `ctan.math.utah.edu` from
`python3 scripts/provision.py worktree .`. The byte-identical archive is served by
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
Its manifest pins the TeX Live 2026 archive, `bibtex.web`, `bibtex.ch`, merged
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
scripts/build-latex-format.sh \
  --engine latex \
  --distribution target/texlive-snapshot \
  --distribution-sha256 61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883
scripts/build-latex-format.sh \
  --engine pdflatex \
  --distribution target/texlive-snapshot \
  --distribution-sha256 61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883
```

Both modes build one clean format and validate the resulting cache image. The
builder reads the common and mode-specific TeX Live input closure from
`tests/latex-source.lock`; its pdfLaTeX configuration is pinned locally in
`tests/latex/pdftexconfig.tex`. The pdfLaTeX representative runtime closure is
separately pinned by path, length, and SHA-256 in
`tests/latex/pdflatex-representative.lock`; the separate representative gate
receives the named format plus only these ten runtime keys. Generated formats
and comparison artifacts remain under `target/` rather than becoming
repository fixtures.
Primary `python3 scripts/provision.py worktree .` additionally constructs the
independent clean-pdfTeX `pdflatex.fmt` under
`target/pdftex14029-reference-format/`. Construction stages the same locked
pdfLaTeX source closure used by Umber, adds only the pinned Web2C configuration
and TCX profile required by the reference executable, selects
`-progname=pdflatex-dev`, and rejects recorder inputs outside the staged closure.
Both the format and its deterministic JSON receipt are native assets, so linked
worktrees receive the exact qualified bytes rather than rebuilding them.

`python3 scripts/check-pdftex-format-pair.py --distribution PATH
--distribution-ahash64 AHASH64` is the focused live gate for that pairing. It
checks the reference receipt and the distribution's schema-12 pdfLaTeX record
against `tests/latex-source.lock`, runs `tests/latex/format-pairing.tex` through
clean pdfTeX and Umber in DVI mode, and requires identical `2026-06-01` and
`proposition` macro markers. Its receipt records both binary and format SHA-256
identities plus the macro-marker fingerprint. It does not run or inspect Umber
PDF.
The source lock also pins the schema-3 distribution digest. Both flags are
required, the local root is authenticated before compilation, and all four
engine runs use the same absolute path and pin with offline resolution.
All builder-started Umber and format-cache subprocesses reuse
`scripts/run-umber-guarded.py` with finite engine fuel, aggregate process-group
RSS and wall-time ceilings, and TERM-to-KILL/reap enforcement. Compiler-only
work remains outside that guard. Tune the bounded builder through the
`UMBER_LATEX_FORMAT_ENGINE_FUEL`, `UMBER_LATEX_FORMAT_MAX_RSS_MIB`, and
`UMBER_LATEX_FORMAT_TIMEOUT_SECONDS` variables rather than writing a separate
watchdog.

Before spending the full authority runtime, validate a newly materialized
pdfLaTeX mirror with independent empty native caches:

```bash
scripts/check-latex-representative-resources.sh \
  --distribution target/texlive-snapshot \
  --distribution-sha256 61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883 \
  --format /path/to/generated/pdflatex.fmt \
  --receipt target/pdflatex-resource-smoke.txt
```

The source-profile smoke prefetches the 64 construction keys and ten runtime
keys; the loaded-format smoke starts from the supplied image and prefetches
only the ten runtime keys. Both runs unset ambient TEXMF search paths, select
the explicit pinned distribution in offline mode, and use distinct empty cache
roots. The optional receipt binds the locks, format, root, key counts, and both
successful outcomes.

The committed Plain-format builder applies the same watchdog contract to both
clean INITEX generations and to the source-loaded and format-loaded DVI runs:

```bash
scripts/build-wasm-plain-format.sh --texmf-dist /path/to/texmf-dist --check
```

Its independent bounds are configurable through
`UMBER_PLAIN_FORMAT_ENGINE_FUEL`, `UMBER_PLAIN_FORMAT_MAX_RSS_MIB`, and
`UMBER_PLAIN_FORMAT_TIMEOUT_SECONDS`.

The complete supported INITEX matrix has a serial entry point with distinct
pinned distribution roots for Plain and LaTeX:

```bash
scripts/build-initex-format-matrix.sh \
  --plain-texmf-dist /path/to/texlive-2025/texmf-dist \
  --latex-texmf-dist target/texlive-snapshot/texmf-dist \
  --latex-distribution target/texlive-snapshot \
  --latex-distribution-sha256 61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883
```

It delegates to the three builders above without overriding their resource
guards and reports success only after Plain, LaTeX, and pdfLaTeX all pass.

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

The DVI corpora under `tests/corpus/math` and `tests/corpus/align` commit TeX
source files plus `.expected.dvi` reference fixtures. The default `umber` cargo
tests run every `.tex` case in those areas against the committed DVI fixtures
without invoking live reference tools.

Three areas have retired into the minifixture system under `umber2-alfh.3`,
because a `.expected.dvi` fixture compares one channel against Umber's own
prior output while a minifixture compares every channel against the pinned
oracle: `tests/corpus/leaders`' six cases became
`command-semantic/page-output`'s `leaders-*` cases, and `tests/corpus/dvi` and
`tests/corpus/page`'s thirty-two became thirty-one cases (one source was
byte-identical in both areas) spread across `page-output`, `math`, and
`alignments` by what each actually exercises rather than by which area it sat
in.

`tests/corpus/canonical-dvi` is what survives of that retirement: two closed
case directories whose `source.tex`/`expected.dvi` pairs back the
canonical-divergence regression tests in
`crates/umber/tests/it/e2e_conformance/canonical_dvi.rs`. It is a static copy, deliberately
outside `scripts/regen-fixtures.sh`'s DVI-area list, because those two tests
pin a specific past divergence rather than tracking the reference engine.

DVI-area regeneration runs the supported `tools/refexec` compatibility CLI,
which delegates its process kernel to fixturegen,
copies the pinned local CM TFMs and case-local support files, uses INITEX for the math
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

`tests/corpus/pdf` commits 15 Git-validated closed case directories containing
minimal primitive-only sources, pinned reference PDFs, deterministic Umber
PDFs, normalized catalog/page/resource/content
structure, exact 72-dpi grayscale PGM renders, and renderer/hash attestations.
Synthetic PDF parser and importer inputs use `ValidPdfFixture`, the
`pdf_writer`-backed adapter in `test_support::pdf_fixture`. The separate
handwritten `RawPdfFixture` is restricted to tests whose evidence requires
classic-xref bytes, malformed syntax, cycles, depth limits, or independence
from `pdf_writer`; complex object-stream syntax remains a committed externally
generated fixture.
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
`scripts/check-pdf-external.sh`. Its qpdf 12.3.2 matrix uses focused native CLI
jobs to produce temporary object-compression, raster, alpha, and DCT artifacts,
then checks those alongside representative classic trailers, imported PDF,
Type 1/TrueType/PK/subset/tagged fonts, annotations,
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

An isolated linked worktree must be provisioned during slot setup with
`python3 scripts/provision.py worktree <worktree>`. The script resolves the
primary checkout from Git's shared worktree metadata and copies missing files
from the explicit `tests/native-test-assets.lock` allowlist. The allowlist
contains only the four oracles and their declared corpus, hyphenation, TFM, and
TRIP/e-TRIP file dependencies; it cannot select a directory. Rust tests only
consume the resulting files and never mutate the checkout to set themselves
up.

When the hosted snapshot root pin is not yet published, primary setup can use
the independently authenticated release-runtime tree:

```bash
python3 scripts/provision.py runtime-source \
  --mirror https://ftp.math.utah.edu/pub/tex/historic/systems/texlive/2026/ \
  --mirror https://mirrors.ibiblio.org/pub/mirrors/CTAN/systems/texlive/Images/
python3 scripts/provision.py worktree . \
  --runtime-source third_party/texlive-20260301-texmf
```

The explicit source path verifies the snapshot lock's selected runtime files
before staging the conformance lock's exact SHA-256 records. The preceding
`runtime-source` command authenticates the release archive, package database,
and runtime source; this local path does not create a hosted distribution or
require R2 credentials.

`crates/umber/tests/it/e2e_conformance/assets.rs`'s `with_gate` remains the
single gate choke point, so an absent oracle cannot be confused with a passing
gate. Its failure points linked worktrees at `provision.py worktree`. When the
primary checkout itself lacks an asset there is nothing to copy from, and the
provisioner names the missing paths and points at
`python3 scripts/provision.py worktree .`.

Every source and destination must match its committed SHA-256. Provisioning
uses an independently verified temporary copy followed by atomic rename, not a
symlink or hard link, so code running in one worktree cannot rewrite the
primary checkout's evidence through the provisioned path. Existing mismatched
files are rejected rather than replaced, missing primary assets produce an
error naming that checkout and the setup command, and successful copies remain
gitignored so `git status` stays clean. A primary-checkout run never searches
another cache or downloads anything; it reports its exact missing allowlist and
requires `python3 scripts/provision.py worktree .`.

Story and Gentle additionally verify their oracle against the
`expected_ref_dvi_sha256` pin in `tests/corpus-manifest.txt` inside
`parity_harness::run_named_fixture_document`, so a stale or foreign oracle fails
with a hash-drift message rather than a confusing DVI mismatch. TRIP and e-TRIP
do not have a normalized-DVI manifest pin inside the Rust harness. Their raw
bytes, like all assets copied between worktrees, are nevertheless pinned by
`tests/native-test-assets.lock`; an intentional regeneration therefore requires
an audited lock update rather than silently distributing one checkout's changed
oracle to every linked worktree.

The manual TeX82 and e-TeX observer scripts publish their reproducible
diagnostic channels under `target/trip-observer-output/<trip|etrip>/`. They do
not write the lock-verified conformance inputs under `target/trip-oracles/`.
Primary provisioning owns the separate promotion step: it verifies the
observer channels against `tests/native-test-assets.lock` and atomically copies
only the locked channels into `target/trip-oracles/`. Running either observer
manually therefore leaves a subsequent worktree provision verification
unchanged. The observer ownership self-test proves this with a synthetic
sealed input and two atomic generated publications; it does not add a second
conformance verdict.

## External Document Corpus

External document inputs live outside committed fixtures. The line-oriented
`tests/corpus-manifest.txt` pins support files and documents by URL, fetched-byte
SHA-256, license determination, and redistributability flag. Runnable documents
also select a format source and pin the reference DVI SHA-256 after DVI preamble
banner normalization.

`python3 scripts/provision.py worktree .` builds `tools/fixturegen` and runs
`--sync-corpus` to fetch or verify those inputs under gitignored
`third_party/corpus/`, then acquires the
remaining local support files and generates all four end-to-end DVI oracles.
For a primary checkout it also runs the TeX82 and e-TRIP observer workflows;
those workflows write generated diagnostics under
`target/trip-observer-output/`, after which provisioning verifies each locked
SHA-256 and atomically promotes the required channels into
`target/trip-oracles/` for linked-worktree copies.
`fixturegen --reference-dvi` directly owns the manifest-bound reference
staging, deterministic invocation, hash check, and atomic publication. The
feature-enabled parity command delegates its live reference half to that same
kernel and retains comparison and triage only.
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
`python3 scripts/provision.py worktree .`. The generated `.expected.dvi` files are
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
`EngineSession` architecture's assembled DVI for `story.tex` and
`gentle.tex` must remain byte-identical to real pdfTeX's output, normalized
only the same way the separately named conformance tests already are. All four
now execute through the same persistent loaded-Plain provider path. Each shares
its exact staged fixture directory
(`parity_harness::run_named_fixture_document`, with the same
`plain.tex`/document/`hyphen.tex`/TFM staging consumed by the shared provider
runner) and its registered gate, so both reach their assets through
`assets::with_gate` and neither can skip silently. See the
[End-to-End Conformance Gate Contract](#end-to-end-conformance-gate-contract)
above for what an absent oracle does.

```bash
cargo test -p umber --test it e2e_conformance_story_canonical -- --nocapture
cargo test -p umber --test it e2e_conformance::e2e_conformance_gentle_canonical -- --exact --nocapture
```

The Gentle oracle is the existing 263424-byte real-pdfTeX artifact, SHA-256
`04f86e97e8264f9b8ce35dc1e9df27f2b075ca85365af71acc5fe1478399866b`.
The canonical Story gate runs in the routine native suite. The canonical Gentle
gate matches its oracle byte-for-byte but is marked `#[ignore]`; it runs only
through the explicit manual command above, not through `cargo test --tests`.
Gentle remains a byte-exact DVI conformance gate when invoked, not an automated
differential-tracer fixture: the tracer's structural tests admit only committed
microfixtures and synthetic fixtures, and do not load Gentle or another full
document.

The shared runner builds the complete pinned Plain recipe, prepares it through
`PreparedFormatProvider`, and supplies each document as a fresh explicit
`PreparedFormatJob`. Construction owns `plain.tex`, `hyphen.tex`, and the
preloaded Plain TFMs; the staged document and its remaining input/font files
become typed job resources. The provider owns format caching, authenticated
worker construction, image loading, and the fresh memory `World`; no family
helper owns an INITEX session, dump/load adapter, staged resource host, or
mutable loaded universe. The separately named Story and Gentle gates differ
only in their retained acceptance observations: the noncanonical-named route
also checks the macro-invocation-provenance budget, while both DVI routes use
the same loaded-format execution substrate.

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

The [command-core diagnostic tools](command_core_diagnostics.md) document the differential tracer, full-document trace generation, worklist accounting, stream alignment, and first-failure locator. For diagnosis order and oracle authority, follow the [Canonical Divergence Working Contract](canonical_divergence_workflow.md).

## TRIP Corpus

The original Knuth TeX82 TRIP and e-TeX V2 e-TRIP workloads are end-to-end DVI
conformance tests governed by the
[End-to-End Conformance Gate Contract](#end-to-end-conformance-gate-contract):
they run when their local inputs and oracles are present and fail with an
actionable report when they are not.

```bash
python3 scripts/provision.py worktree .
python3 scripts/provision.py worktree . --offline
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_trip_canonical -- --exact --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_etrip -- --exact --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_gentle_canonical -- --exact --ignored --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
```

`python3 scripts/provision.py worktree .` acquires the shared hyphenation and font
inputs, reads `tests/trip-manifest.txt`, tries each entry's locators in declared
order, fetches exact official TRIP and e-TRIP bytes into gitignored
`third_party/trip/`, and verifies every candidate against the entry SHA-256
before acceptance. The tests
use the pinned canonical `trip.tfm`, then run the documented INITEX and
format-loaded TRIP phases in process.

Cargo conformance tests do not launch Umber as a subprocess. Story and Gentle
call the engine directly through the staged fixture callback. The ignored
`e2e_conformance_trip_canonical` probe uses retained
`EngineSession`, `World` roots, and typed resource fulfillment for
both phases without an alternate command/input fallback; TRIP and e-TRIP share
the surrounding two-phase fixture helper.
`scripts/check-and-test.sh` preflights the gitignored e2e oracles before
starting the workspace gate and warns that absent ones will fail their gates.

The exact ignored Cargo tests above are the sole conformance owners;
provisioning and fixture regeneration do not establish correctness. Canonical
semantic, transcript, log, status, and effect channels gate where present. The
DVI oracle normalizes only the preamble comment and otherwise requires byte
identity with the locally pdfTeX-generated fixture. Comparator failure
detection remains ordinary focused unit coverage, not a separate parity gate.
Regeneration executes the two-phase workload from `trip.tex` and
`trip.tfm` and never copies the official `third_party/trip/trip.dvi`.

The e-TRIP gate also consumes the pinned official V2 `etripin.log`,
`etrip.log`, `etrip.fot`, `etrip.typ`, and `etrip.out` masters. The exact e-TeX
2.6 oracle comparisons run first. The official text layer then applies the
bounded contract documented in [TRIP](trip.md): platform framing and the
manual-listed numeric allowances, the licensed two-line source adaptation,
and three explicit V2-to-2.6 profile bridges. The output file remains byte
exact. A typed DVItype projection compares every integer framing/page field
while excluding only the tool banner, option rendering, floating-point
pixels-per-unit value, and preamble comment; no host `dvitype` executable runs
inside Cargo tests.

Official `trip.typ` is identity-pinned diagnostic input. It is DVItype output,
not an Umber artifact, and the upstream Web2C comparison applies
platform-numeric tolerance filtering. It is not a second acceptance oracle for
the same DVI bytes. Generated terminal photos and `tripos.tex` also remain in
the diagnostic tier. The exact ignored Cargo command above is the sole
maintained Gentle audit; no wrapper script owns this comparison.

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
production distribution: `python3 scripts/provision.py snapshot` enforces full
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
