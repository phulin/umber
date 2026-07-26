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
   (1.40.27) are the sole authority for correct behavior. Every fix cites a
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
   remain a migration *target* — code being deleted or routed away from —
   but never a source of expected results. Do not consult its behavior to
   decide what canonical code should do.

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
   cargo run -q -p tex-command-stream -- --repository .
   ```

   Run this from the repository root. It is hermetic (no corpus,
   distribution, or live TeX tool required) and never invokes a reference
   engine.
   - Exit `0`: no divergence in the committed fixture registry.
   - Exit `1`: prints the earliest ordered divergence -- `fixture <name>
     diverged at event <index>` -- followed by the expected event, the actual
     observed event, and source context.
   - Exit `101`: a Rust panic reached before any semantic mismatch; the panic
     message and `file:line` (rerun with `RUST_BACKTRACE=1` for a full
     backtrace) is itself the diagnosis and takes priority over a
     stream-mismatch report because it is reached first.

2. **First-failure locator next**, for the live end-to-end front, only when
   the tracer's fixture registry doesn't cover the failing input (for
   example, it needs live document/font/hyphenation material outside
   `tests/corpus/command`):

   ```bash
   cargo run --profile test -p umber --example first_failure_locator -- gentle
   cargo run --profile test -p umber --example first_failure_locator -- story
   ```

   It reports the live execution mode and the first `ExecError`/
   `CanonicalSessionError` (with provenance-resolved source context) or Rust
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

## 5. Standing gates

Run before closing any divergence issue: `cargo test -q --tests`,
`cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`.
`scripts/check.sh` runs the fmt+clippy gate without rerunning tests; use it
once tests have already passed. `e2e_conformance_story_canonical` (see
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

Reference DVI generation: `scripts/setup-conformance-tests.sh`, with real
pdfTeX 1.40.25 on `PATH`; `UMBER_REF_TEX` selects a specific binary.

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
  `cargo test -q --tests`, `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, and the byte-exact
  `e2e_conformance_story_canonical` test.
- **divergence**: a point where Umber's canonical behavior differs from an
  oracle -- a semantic event mismatch reported by the differential tracer, or
  a DVI byte mismatch reported by the parity harness.
- **first-failure locator**: `crates/umber/examples/first_failure_locator.rs`
  (§2 step 2). Runs a document through the canonical engine end-to-end and
  reports where execution first stopped -- an error or a panic. It proves
  execution stopped, not that output is wrong: a run that completes is not
  proof of correctness, only silence past that point.
- **target**: code being deleted or routed away from during migration -- the
  retired `Executor`/`InputStack`, `tex-lex`, `tex-expand` path (§1.3). A
  target may be inspected to understand what is being replaced, but is never
  a source of expected behavior.
