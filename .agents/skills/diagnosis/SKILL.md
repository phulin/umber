---
name: diagnosis
description: Diagnose Umber TeX-engine, format, resource-replay, and corpus failures by reducing symptoms to the earliest canonical semantic divergence. Use for timeouts, fuel or RSS growth, scanner and recovery errors, alignment or brace-state defects, macro lifetime failures, format-only failures, flaky provenance bounds, and clusters of arXiv rows that need generic TeX/pdfTeX fixes rather than document-specific patches.
---

# Diagnosis

Find the first incorrect engine transition, prove its canonical semantics, and
fix the owning invariant. Treat the corpus row as a reproducible integration
fixture, not as the specification.

## Diagnostic Workflow

### 1. Establish an exact reproduction

- Record the binary, format, distribution, source/archive, working directory,
  engine mode, offline setting, and every guard.
- Prefer the optimized test-profile binary and serial execution.
- Preserve failed builder workspaces and per-row outcome artifacts.
- Do not raise time, fuel, RSS, or error guards unless evidence proves the work
  is finite, semantically correct, and legitimately exceeds the contract.

### 2. Find the earliest divergence

- Trace backward from the terminal symptom until the first state, token,
  ownership, or command-demand transition differs from the intended behavior.
- Instrument one canonical variable or boundary at a time.
- Prefer bounded probes that render resolved control-sequence names, input-stack
  frames, macro argument boundaries, replay transitions, provenance ownership,
  alignment state, and rollback epochs.
- Remove diagnostic instrumentation before committing.
- Do not patch the final error site when an earlier transition corrupted it.

### 3. Audit canonical sources

- Read the relevant `tex.web` sections directly. Check `pdftex.web` for
  divergence and the owning LaTeX/package source when behavior is above the
  engine layer.
- Map each canonical operation to exactly one Umber owner and every correction,
  backup, suspension, rollback, or recovery path.
- Prefer one coherent state invariant. Remove duplicate owners or shadow state
  when the canonical model has one owner.
- Record exact source sections in tests or durable architecture documentation.

### 4. Reduce and challenge the hypothesis

- Create the smallest subsystem-level reproducer for the first divergence.
- Add negative controls that preserve the suspected surface syntax while
  removing earlier history, resource suspension, nesting, or recovery.
- Compare source versus loaded-format execution when serialization is suspected.
- Separate aliases from physical tokens, raw delivery from expansion, and local
  records from process-global allocators.
- Reject a hypothesis when the reduced transition passes; do not add metadata,
  thresholds, or special cases to preserve it.

### 5. Implement the owning invariant

- Implement the TeX/pdfTeX semantic rule, not a paper, package, or corpus-row
  workaround.
- Make invalid duplicate ownership structurally difficult or impossible.
- Preserve existing guards and recovery contracts.
- Keep the issue scoped to its owned transition. File a linked Beads issue for
  a distinct later failure rather than absorbing it.

### 6. Validate from narrow to broad

Run, in order:

1. The new focused regression.
2. Owning crate tests with `cargo test -q --tests`.
3. Related cross-crate tests.
4. The full serial native suite when shared state or test ordering is relevant.
5. `scripts/check.sh` or `scripts/check-and-test.sh`.
6. The exact guarded row or cohort with pinned offline artifacts.

Repeat order-sensitive gates. Treat parallel-only failures as possible shared
global-state interference; diagnose the owner rather than weakening bounds.

### 7. Preserve durable evidence

- Put status, reductions, negative controls, rejected hypotheses, and exact
  last-known-good/first-known-bad transitions in Beads.
- Update `docs/` when behavior, ownership, or architecture changes.
- If blocked, leave the issue open, remove instrumentation, confirm a clean
  worktree, and write the next reduction boundary so another session can resume.

## High-Value Tools

- Direct `tex.web`, `pdftex.web`, and package-source inspection.
- `rg` for bounded ownership and call-site audits.
- `git show` and commit-specific binaries for behavioral bisection.
- Per-row `outcome.json`, corpus receipts, and exact artifact hashes.
- Temporary semantic state probes and resolved `tex-command` input-level dumps.
- Provenance/accounting views such as `ProvenanceStats` and `OriginKeyRuns`.
- Focused Rust regressions followed by serial workspace gates.
- Beads comments and related closed issues for prior evidence.

## Anti-Patterns

- Treating all timeouts, RSS exits, undefined controls, or EOF errors as one bug.
- Debugging mainly from wall time or the terminal diagnostic.
- Deeply tracing every paper in a corpus before clustering.
- Persistent broad logging instead of a bounded semantic flight recorder.
- Package-specific macro injection or corpus-specific recovery.
- Raising limits, subtracting accounting, or weakening a test contract.
- Adding replay metadata without proving the canonical semantic need.
- Assuming format serialization, package version, or store capacity without
  source/loaded comparisons and negative controls.
