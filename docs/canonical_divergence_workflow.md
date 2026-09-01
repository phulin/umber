# Canonical Divergence Working Contract

Status: contract, not narrative. Read once; every rule below is durable and
applies to any `umber2-johp` divergence, regardless of which primitive, mode,
or fixture is involved. It carries no issue-by-issue history — an issue ID
appears only parenthetically, as provenance for a rule that would otherwise
look arbitrary, never to narrate what a past agent did or found.

Scope: working a semantic or byte-level divergence between Umber's canonical
`tex-command`/`tex-exec` command core and TeX82/e-TeX/pdfTeX, under the
`umber2-johp` epic (`bd show umber2-johp` lists current children).

## 1. Oracle hierarchy

1. tex.web (TeX82), `etex.ch` (e-TeX 2.6), and pdfTeX's WEB/change files
   (1.40.29) are the sole authority for correct behavior. Every fix cites a
   numbered section from one of them.
2. Evidence for whether Umber matches that authority comes only from the
   pinned `tex-oracle` semantic command-transition traces (compared by
   `tools/tex-command-stream`) and byte-exact reference DVI from a real
   pdfTeX build (compared by `tools/parity-harness`). See
   `docs/tex_command_core.md` §31 for how those instrumented reference
   engines are built and regenerated.
3. The retired Umber implementation (`Executor`/`InputStack`, `tex-lex`,
   `tex-expand`) is never an oracle for expected behavior, at any step, for
   any reason (`docs/tex_command_core.md` §35.7 and §4's non-goals). It may
   remain a migration _target_ — code being deleted or routed away from —
   but never a source of expected results. Do not consult its behavior to
   decide what canonical code should do.

### Reading an oracle trace by event index

The tracer's event index equals the `sequence` field of the oracle event, which
is the `events.jsonl` line number **minus 2**. To look up the divergence the
tracer reports at event `N`, read line `N + 2`. Verify the mapping against two
independent divergences before relying on it; it has been stated inverted in a
dispatch prompt, and an off-by-two lands on a neighboring event that is usually
plausible enough to reason from without noticing.

Every event index is relative to the commit it was measured on. An index quoted
from a bd issue body, a dispatch prompt, or an agent report is only valid
against that report's base: composing two fixes during a rebase moves later
indices. Re-measure on your own base before relying on one.

## 2. Diagnosis order

Fixed order; do not skip a step or substitute ad hoc instrumentation for it.
Do not start with hand-added `eprintln!`/debug-panic instrumentation or
another ad hoc reproduction: both tools below already report a
source-attributed divergence or a Rust panic origin, and skipping them wastes
a run re-deriving what they would have named directly. The retired Umber
implementation (`Executor`/`InputStack`, `tex-lex`, `tex-expand`) is never an
oracle at any step (§1.3); consult it only as a migration target being routed
away from.

1. **Differential tracer first**, for the earliest ordered semantic
   divergence against committed fixtures:

   ```bash
   cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- --repository . --max-divergences 100000
   ```

   Run this from the repository root. It is hermetic (no corpus,
   distribution, or live TeX tool required) and never invokes a reference
   engine. `cargo run-dev` selects the repository's optimized test profile;
   do not substitute `cargo run`, whose target/debug replay is prohibitively
   slow for full-document traces. Full-document tracing remains a manual
   diagnostic tier and must not be added to automated correctness gates.
   Every run that happened prints its report and ends with a `VERDICT:` line
   naming the outcome and the status carrying it. Read the status before the
   totals; only two of the four mean the totals are exact.
   - Exit `0` (`CLEAN`): every registered fixture was compared to exhaustion
     and none diverged.
   - Exit `1` (`DIVERGED`): every registered fixture was compared to
     exhaustion, so the divergence total is exact. Prints the divergence
     total, the root-site total, a per-fixture accounting, and then the
     ordered worklist -- `fixture <name> diverged at event <index>` followed
     by the expected event, the actual observed event, and source context.
     Exact recurrences of one root site are collapsed into one entry;
     `--ungrouped` prints one entry per divergence. Compare a fix against the
     divergence total, not the root-site total. See "Grouped worklist and run
     accounting" in [Testing Infrastructure](testing_infrastructure.md).
   - Exit `2` (`PARTIAL`): a registered fixture was never compared (its
     document trace is not generated on this checkout) or a fixture's
     comparison stopped at its `--max-divergences` budget. Every total is a
     LOWER BOUND, and a total of `0` does not mean convergence. The header
     says so in place of the "compare against historical totals" instruction
     a complete run gives, and the bounded fixture's `BOUNDED:` notice names
     both its totals as floors. Never rank or dispatch from a partial
     worklist: run `scripts/build-tex82-document-traces.sh` and/or raise the
     budget first.
   - Exit `3`: the run could not be performed at all -- a usage error, an
     unreadable suite, or a document registry inconsistent with its committed
     pin. Nothing was compared.
   - Exit `101`: a Rust panic reached before any semantic mismatch; the panic
     message and `file:line` (rerun with `RUST_BACKTRACE=1` for a full
     backtrace) is itself the diagnosis and takes priority over a
     stream-mismatch report because it is reached first.

   `--max-divergences N` bounds **ordered divergences** per fixture -- not
   root sites and not printed entries, so a bounded grouped run prints fewer
   than `N` entries. One divergence per fixture, the contained replay failure,
   is reported outside the budget, so a bounded fixture's total can exceed `N`
   by one. See "What the budget counts" in
   [Testing Infrastructure](testing_infrastructure.md) for why the unit is
   ordered divergences and not root sites.

   The default budget (`20`) saturates on `gentle` and returns `PARTIAL`,
   which is why the command above passes `--max-divergences 100000`: an
   exhaustive run is the only one whose totals may be compared against a
   figure recorded elsewhere. Raise the budget rather than dispatching from
   the floor.

2. **First-failure locator next**, for the live end-to-end front, only when
   the tracer's fixture registry doesn't cover the failing input (for
   example, it needs live document/font/hyphenation material outside
   `tests/corpus/command`):

   ```bash
   cargo run --profile test -p umber --example first_failure_locator -- gentle
   cargo run --profile test -p umber --example first_failure_locator -- story
   ```

   It reports the live execution mode and the first `ExecError`/
   `SessionError` (with provenance-resolved source context) or Rust
   panic it hits. As a first-failure locator (Glossary), it can only show
   that execution stopped, not that completed output is wrong. `story`
   currently completes cleanly and is a regression gate (§5): a new `story`
   failure is a regression to fix immediately, not the divergence under
   investigation.

3. **Manual instrumentation only if both come up short.** Reach for
   `eprintln!`/debug-panic probes or a custom reproduction only after the
   tracer's fixtures do not exercise the failing input and the first-failure
   locator does not reproduce it with actionable context. Keep any such
   instrumentation temporary and remove it once the tracer or locator
   confirms the fix.

Tool descriptions -- exact staged inputs, build-profile rationale, and what
each command's output looks like -- live in "Canonical Command-Core
Diagnostics" in [Testing Infrastructure](testing_infrastructure.md). Read
them there; this document does not restate them.

## 3. Fix discipline

- Every fix's commit message and bd close comment cites a numbered
  tex.web/e-TeX/pdfTeX section.
- Implement the generic behavior that cited section describes. Never
  special-case a fixture, source file name, or specific token sequence to
  make one input pass.
- If the tracer cannot observe the defect, add the smallest canonical
  semantic or geometry event that exposes it and a bounded committed
  microfixture that proves the event. Permanent automated tracer fixtures
  are microfixtures only: never add Gentle, TRIP, e-TRIP, or another full
  document to the routine tracer gate. Full-document traces remain a manual
  diagnostic tier (§2).
- The dispatch-completeness invariant (`docs/tex_command_core.md` §33.2)
  applies to every `UnexpandablePrimitive` in `scan_command`: it is routed by
  a named arm or fails loudly via `ExecError::UnimplementedPrimitive`. Never
  reintroduce a silent catch-all that treats "no dispatch arm" as "succeeded
  and consumed nothing."

## 4. One divergence at a time; placeholder successors

- Fix exactly one divergence per issue, even if a later one is already
  visible. Investigation and fix must not span two agents in one pass.
- Before closing, file the next divergence as a new bd issue titled
  `Investigate next <front> canonical divergence after <this-issue-id>`. The
  title names no root cause.
- The body carries only OBSERVED facts: the exact tracer/locator output,
  execution mode, the failing token or meaning, the error identity (type and
  fields), file:line, and the exact reproduction command.
- Any theory about the cause goes under an explicit heading
  `UNVERIFIED SPECULATION -- discard freely`, or is omitted entirely. Do not
  elevate a guess to a "strong hint" for the next dispatch — a confident
  wrong title costs the next agent more than no title does.
- The next agent renames the issue once the real cause is known, and
  re-derives any theory itself rather than trusting the placeholder's guess.
- Keep only one active issue and one implementation agent per divergence
  front. A successor is a placeholder for the newly observed front, not a
  second speculative implementation stream.
- After a completed branch is integrated, remove its worktree and local
  branch. Preserve unfinished or conflicting work by attaching its exact
  disposition to the owning bd issue before cleanup, not by leaving an
  unowned worktree or coordinator handoff.

## 5. Standing gates

Run before closing any divergence issue: `cargo test -q --tests`, then
`scripts/check.sh`, which runs the format and lint gates without rerunning
tests. Report its final verdict line.

Never substitute a hand-written `cargo clippy` for the clippy gate, and never
report "clippy clean" from one. A bare `cargo clippy` leaves warn-level lints
at warn and exits 0 on a real violation, and any single invocation lints one
feature resolution, while the gate lints the declared set of resolutions the
tree is actually built in -- so a hand-written run can be green while the gate
is red, and vice versa. To run one gate on its own, name it:
`scripts/check.sh clippy` runs the identical passes the full run uses. What
those passes cover, and how a known-dirty configuration is quarantined against
its issue, is recorded under "What The Clippy Gate Covers" in
[Testing Infrastructure](testing_infrastructure.md); a warning you can
reproduce by hand but the gate does not report is itself a gate defect to file.
`e2e_conformance_story_canonical` (see
"Canonical Story Regression Gate" in
[Testing Infrastructure](testing_infrastructure.md)) must stay byte-exact
against real pdfTeX output — never weaken or skip that comparison. Its live
proxy, `first_failure_locator -- story`, must keep completing without a
failure; a new `story` failure is a regression to fix immediately, not the
divergence under investigation.

The tracer and locator fronts advance every time a divergence is fixed, so no
fixed event index or issue number is pinned here. Before starting, run
`bd show umber2-johp` (or `bd ready`) to find the current open successor for
the front you're diagnosing, and compare its recorded OBSERVED output against
a fresh run of the same command to confirm you're looking at the same
divergence, not a new one.

Reference DVI generation: `python3 scripts/provision.py worktree .`, using the
pinned instrumented pdfTeX 1.40.29 build from TeX Live 2026.

### Recent-arXiv document front

A recent-arXiv document enters the canonical DVI parity front only after the
complete, unmodified source compiles cleanly with the pinned TeX Live 2026
pdfTeX in both DVI and PDF modes. Record the archive, entrypoint, oracle,
format, output, and page-count identities for both qualification runs before
running Umber. PDF compilation is only an eligibility check during this pass;
record PDF-only or otherwise non-DVI-capable rows for the later PDF pass.

Run the complete source once in Umber's DVI mode with 500,000,000 expansion
fuel, the ordinary 10,000,000 execution-step cap, and the standing wall-time,
RSS, and termination-grace guards. Expansion fuel is only a runaway guard:
ordinary consumption differences have no parity significance. Compare the
result with `parity-harness --compare-existing-dvi`, whose only DVI
normalization is the preamble comment, and diagnose the first meaningful
semantic or DVI divergence. Do not search source prefixes or recompile one
page at a time.

Preserve the source-derived TeX jobname in every qualification, reference, and
Umber run. It is the entrypoint basename without `.tex`, as emitted by
`python3 scripts/arxiv_corpus.py jobname ENTRYPOINT`; record it in the row
identity. Never pass `--jobname` merely to give an oracle artifact a stable
label: TeX and LaTeX use `\jobname` to find archive-provided `.bbl`, `.aux`,
and other side files. Rename or copy the DVI only after the engine exits.

When one document's complete DVI is canonically exact, advance directly to the
next eligible source in the locked corpus and repeat the full-source DVI run.
Do not run, inspect, render, or use Umber PDF output for diagnosis until the
corpus-wide DVI pass is complete. The later PDF pass will compare the same
qualified papers in corpus order and ignore equivalent serialization,
font-subset tags, and extractor rounding. This corpus-wide DVI-first,
PDF-second order prevents PDF finalization details from hiding an earlier TeX
typesetting divergence.

## 6. Never half-implement, never weaken to get green

- Do not silently half-implement a primitive or scanner arm just to unblock
  the fixture in front of you. Either implement the cited section's full
  behavior, or fail loudly (§3) and file the remainder as its own bd issue.
- Never weaken, delete, or loosen an assertion — a debug check, a gate
  threshold, a byte-exact comparison — to make a test pass. If the assertion
  is firing on a real bug, fix the bug. If the assertion itself is wrong,
  that is a separate, explicitly justified change, never a side effect of an
  unrelated fix.

## 7. A/B comparison

Never use `git stash` to compare "with fix" and "without fix" behavior —
state under it is easy to lose or misattribute. Use a second git worktree
(`git worktree add <path> <branch>`) or a saved patch file
(`git diff > patch`, then `git apply`/`git apply -R`) instead.

## Glossary

Terms this epic uses without defining them, in the sense used throughout this
document and [Testing Infrastructure](testing_infrastructure.md):

- **canonical**: the `tex-command`/`tex-exec` command-core architecture and
  its behavior, as defined solely by tex.web/e-TeX/pdfTeX (§1.1). "Canonical"
  never refers to the retired Umber implementation (§1.3).
- **oracle**: the pinned reference used as ground truth for canonical
  behavior -- either the `tex-oracle` semantic command-transition traces
  (compared by the differential tracer, `tools/tex-command-stream`) or
  byte-exact reference DVI from a real pdfTeX build (compared by
  `tools/parity-harness`). See `docs/tex_command_core.md` §31 for how these
  are built and regenerated.
- **gate**: a check that must pass before a divergence issue closes (§5):
  `cargo test -q --tests`, `scripts/check.sh`, and the byte-exact
  `e2e_conformance_story_canonical` test.
- **divergence**: a gating point where Umber's canonical behavior differs from
  an oracle -- a command semantic event mismatch reported by the differential
  tracer, or a DVI byte mismatch reported by the parity harness. Geometry-v2
  differences remain reported and countable advisory diagnostics and do not
  change conformance acceptance.
- **first-failure locator**: `crates/umber/examples/first_failure_locator.rs`
  (§2 step 2). Runs a document through the canonical engine end-to-end and
  reports where execution first stopped -- an error or a panic. It proves
  execution stopped, not that output is wrong: a run that completes is not
  proof of correctness, only silence past that point.
- **target**: code being deleted or routed away from during migration -- the
  retired `Executor`/`InputStack`, `tex-lex`, `tex-expand` path (§1.3). A
  target may be inspected to understand what is being replaced, but is never
  a source of expected behavior.
