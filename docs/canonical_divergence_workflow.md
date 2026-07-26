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

Fixed order; do not skip a step or substitute ad hoc instrumentation for it:

1. **Differential tracer** (hermetic, no corpus/distribution/font setup):
   `cargo run -q -p tex-command-stream -- --repository .`. Exit 0: no
   divergence in the committed fixture registry. Exit 1: prints the earliest
   ordered divergence. Exit 101: a Rust panic reached before any mismatch —
   the panic itself is the diagnosis.
2. **`canonical_probe`**, only when the tracer's fixture registry doesn't
   cover the failing input (for example, it needs live document/font/hyphenation
   material): `cargo run --profile test -p umber --example canonical_probe --
   gentle` (or `story`).
3. **Manual instrumentation** (`eprintln!`, debug panics, ad hoc repros) only
   after both of the above come up short. Remove it once the tracer or probe
   confirms the fix.

Full recipe, exact output shapes, and rationale: "Diagnosing A Canonical
Divergence" in [Testing Infrastructure](testing_infrastructure.md). Read it
there; this document does not restate it.

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
- The body carries only OBSERVED facts: the exact probe/tracer output,
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
proxy, `canonical_probe -- story`, must keep completing without a divergence;
a new `story` failure is a regression to fix immediately, not the divergence
under investigation.

The tracer and probe fronts advance every time a divergence is fixed, so no
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
