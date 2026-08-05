# Generated-input correctness and editor stabilization

> **Status:** implemented and release-gated. Accepted engine generations retain
> bounded positive and authoritative-negative input observations, expose a
> versioned native/WASM projection, and validate retained dependencies against
> the candidate VFS snapshot before reuse. The native TeX-only fixed-point
> coordinator, provisional editor session, WebAssembly representation adapter,
> and direct/worker authored JavaScript facades expose the complete lifecycle.
> `tex-incr` provides the unchanged-root external-input-delta candidate that
> restores only `JobStart` while retaining accepted output history. Native,
> WebAssembly, incremental-fuzz, rollback, oscillation, and balanced optimized
> gates cover the contract.

This document defines how persistent editor compilation composes
root-buffer edits, generated TeX inputs such as `.aux` and `.toc` files, and
bounded fixed-point iteration. It complements
[`incremental_v1.md`](incremental_v1.md),
[`incremental_memoization.md`](incremental_memoization.md),
[`persistent_compile_sessions.md`](persistent_compile_sessions.md), and
[`umber_vfs.md`](umber_vfs.md).

The governing correctness criterion is simple:

> An accepted editor pass must be byte-identical to a cold pass of the same
> root revision against the same incoming VFS snapshot.

Reuse is optional. A generated-input change may reduce reuse, but it must never
permit execution from state derived from different input bytes.

## Current architecture

Umber currently has three relevant mechanisms.

### Retained single-pass editor sessions

`VirtualCompileSession` owns one `tex-incr::Session`. After a root patch it
selects the latest retained named checkpoint before the edit, executes forward,
and adopts the old suffix when a mapped boundary reaches the same canonical
future-state identity. Detached effects and committed artifacts are spliced
separately and are deliberately excluded from the convergence identity.

This is a prefix plus convergent-suffix design, not independent page
compilation. With a retained checkpoint and later state convergence, work often
corresponds to the affected pages. That is an outcome, not an API guarantee:
checkpoint pruning may force an earlier restart, equal line counts do not prove
state convergence, and a reflow may either remain divergent or rejoin at a
later boundary.

### Multipass project sessions

`LatexProjectSession` runs bounded fixed-point jobs. It executes TeX over a
candidate generated generation, optionally executes a bibliography backend,
compares generated signatures, detects oscillation, and accepts the root,
generated files, and final output atomically. `TexFixedPointSession` exposes
the same machinery without bibliography configuration or detection.
The WebAssembly `ProjectSession` exposes the same state machine.

`EditorCompileSession` composes the retained single-pass session with the
TeX-only fixed-point coordinator. Its `advance` operation publishes exactly one
latency-critical pass; `stabilize_attempt` then runs unchanged-root private
passes explicitly and atomically installs the converged session. The low-level
WebAssembly `EditorSession` and authored direct/worker facades preserve the
same split without duplicating pass policy in JavaScript. Bibliography remains
an opt-in `LatexProjectSession`/WebAssembly `ProjectSession` surface.

## Generated-input validation

The retained single-pass path now assembles the candidate's private root/VFS
generation and opens its stage snapshot before constructing an edited engine
candidate. Positive and authoritative-negative dependencies from the accepted
generation are compared with that exact immutable snapshot. Only an exact
match enters ordinary restart selection and checkpoint restore.

Generated outputs are published into the accepted VFS generated layer after a
successful pass. Consequently, the next patch can observe a different
generated input in its incoming stage while a retained checkpoint still
contains state derived from the earlier input outcome.

There are three required transition classes:

- `Missing -> Present(hash)`: the accepted pass probed or attempted an absent
  generated input and later produced it.
- `Present(old_hash) -> Present(new_hash)`: the accepted pass read one
  generation and later published another.
- `Present(hash) -> Missing`: an earlier generated input is no longer present
  in the accepted generation.

Successful `InputRecord`s continue to provide content-addressed source and
provenance identity. A separate copy-on-write `World` dependency map
reduces semantic observations by canonical path, retaining successful reads
and authoritative misses with their access class across checkpoint forks and
restores the prior map root on rollback. The VFS resolver records a lookup
only after it resolves to immutable bytes or authoritative absence; unresolved
resource waits and speculative prefetch hints never enter the map. A mismatch
selects a private edited candidate that executes from `JobStart` and retains
the accepted generation until all
engine, output, and VFS publication checks succeed.

The sharp failure sequence is:

1. revision N reads or probes a generated input near job start;
2. revision N publishes a different outcome for that path;
3. revision N+1 edits root text after the input site;
4. restart restores a later checkpoint without repeating the input operation;
5. execution continues with state derived from revision N's incoming bytes,
   although a cold revision N+1 would use the newly accepted VFS binding.

No accepted output may depend on this sequence being harmless.

## Input dependency contract

Every accepted incremental generation must retain the external input outcomes
that can affect restored state:

```text
InputDependency {
    canonical path,
    outcome: Present(ContentHash) | Missing,
    access class: required read | authoritative probe,
}
```

Speculative prefetch misses are not semantic dependencies. A failed required
read that suspends for resources is candidate state, not an accepted
`Missing` outcome. An authoritative unavailable response or a completed TeX
existence probe is semantic and must be recorded.

The retained map is deterministically ordered, capped at 8,192 distinct paths
per engine `World`, and charged to generation/session retention accounting.
Duplicate observations do not grow it; required reads dominate probes for the
same path, while a later authoritative outcome replaces the earlier outcome.

### Public accepted-observation projection

`VirtualCompileSession::accepted_input_observations` and
`LatexProjectSession::accepted_input_observations` expose schema version 1 of
the same accepted Rust state. Each record carries the canonical VFS path,
authored/generated/distribution namespace, present content hash or authoritative
missing outcome, required-read/probe access class, typed resource kind, phase,
logical revision, optional project pass, optional proven requesting source, and
a typed subsystem owner. An absent requesting source is intentional; adapters
must not manufacture source attribution.

The single-pass ledger describes the accepted TeX revision. The project ledger
accumulates successful TeX and bibliography selections for every pass, including
classic database/style inputs and auto-detection probes, and is published with
the project transaction. Candidate suspension, prefetch, cancellation, failure,
and oscillation never replace the prior accepted ledger. The public project
ledger is capped at 65,536 records.

The WASM `acceptedInputObservations` getter uses the same contract on both
session classes. The authored package copies the ledger onto the completed
output before disposing its one-shot session, so direct and worker consumers
can build dependency graphs without source scanning. Consumers must compare
`schemaVersion` with the versions they understand; an unknown version requires
the existing complete-snapshot/cold fallback, not a best-effort interpretation.

Before selecting or restoring a checkpoint for a new pass, the host integration
must validate these dependencies against the exact immutable VFS snapshot that
the candidate's resolvers will read. Validation must finish before any candidate
engine, VFS stage, effect, or accepted history mutation.

Only inputs actually read or authoritatively observed need validation. A
generated output that is never consumed is not a reason to discard incremental
history. Immutable user and resolved-resource bindings remain subject to their
existing no-rebind contracts; generated inputs are the expected source of
between-pass deltas.

### Mismatch behavior

A mismatch is not a patch error. It selects a safe `JobStart` execution path
against the new snapshot. It must preserve:

- the previously accepted revision and output until the candidate succeeds;
- resource suspension and retry of the same private candidate;
- candidate VFS isolation and atomic generated-output publication; and
- cold-equivalent diagnostics, effects, artifacts, DVI/PDF, and generated
  files.

Candidate retry also preserves the root file's representation. In particular,
an editor session initialized from legacy 8-bit bytes must rebuild its private
VFS root with the session's byte-projection encoder after every resource
response; it must not accidentally publish the UTF-8 encoding of the internal
editor string. Native byte slices and WebAssembly `Uint8Array` inputs therefore
retain the same accepted root bytes and content identity across suspension.

The public editor revision remains the root-buffer revision. Internal
stabilization passes over an unchanged root must not invent observable editor
revisions merely to satisfy engine bookkeeping.

## Stabilization contract

Interactive display and fixed-point completion have different latency needs
and should be separate operations.

### Hot path

An editor patch performs one incremental TeX pass and may publish its output as
provisional display state. Cross-references may reflect the generated inputs
accepted before that pass. The result reports whether relevant generated
bindings changed and therefore whether stabilization is pending.

Suggested host-visible state is:

```text
Provisional { revision, output, stabilization_required }
Stabilizing { revision, completed_passes }
Stable { revision, output, passes }
```

The ordinary `advance` operation remains bounded to one TeX pass. Hosts may
request stabilization on idle, save, export, or another explicit policy
boundary.

### Off-hot-path stabilization

Stabilization repeatedly compiles the unchanged root against the latest private
generated generation until the selected generated-input signature is stable.
`TexFixedPointSession` provides this bibliography-free native coordinator and
reuses the existing project-session rules for:

- private generated generations;
- deterministic signature comparison;
- bounded pass and attempt counts;
- non-adjacent oscillation detection;
- resource suspension without pass reconstruction; and
- atomic publication of stable root, output, and generated files.

The coordinator uses fresh cold TeX sessions for stabilization passes and
retains a suspended session across resource responses. A failure leaves the
last accepted stable state intact and returns a typed error; it does not
partially publish a later generated generation. `EditorCompileSession` retains
the provisional display and prior stable output separately, reports
`Provisional`, `Stabilizing`, or `Stable`, and installs the converged TeX
session and generated generation together. Cancellation or a newer root patch
drops only the private stabilizer.

Generated-byte equality after the provisional pass is already sufficient to
avoid an unnecessary rerun. A label-table parser is neither required nor a
safe substitute for generated-input equality because aux files may execute
arbitrary TeX state changes beyond `\r@...` definitions.

## Incremental stabilization passes

An external-input mismatch starts a private candidate from `JobStart` against
the new immutable VFS snapshot. The candidate executes ordinary expansion,
paragraph construction, page building, and shipout. It retains the accepted
revision and output for atomic failure recovery, but carries no finished-line
or paragraph transaction history into the new execution. Named checkpoints are
republished by ordinary execution.

## Implementation record

### Phase 1: generated-input correctness

The implemented retained-session gate:

1. Covers missing-to-present,
   present-to-changed, and present-to-missing generated inputs.
2. Covers both required reads and authoritative `\openin`/existence probes.
3. Compares the accepted incremental diagnostics, effects, artifacts, DVI/PDF,
   and generated files with a cold pass against the same incoming snapshot.
4. Records positive and negative input dependencies and validates them against
   the incoming VFS snapshot before candidate mutation.
5. Executes mismatches safely from `JobStart` and accepts atomically.

The exit criterion is satisfied: generated-input changes can reduce reuse but
cannot change any accepted observable relative to cold execution.

### Phase 2: general editor stabilization

1. `umber::TexFixedPointSession` and the shared `FixedPointLimits` policy run
   TeX-only jobs through the project convergence machinery without bibliography
   configuration.
2. `EditorCompileSession::advance` keeps the one-pass incremental result
   available as provisional display.
3. Native `stabilize_attempt`, `status`, `display_output`, and `stable_output`,
   plus the low-level WebAssembly `EditorSession` and retained direct/worker
   facades, expose provisional, stabilizing, stable, resource-wait,
   cancellation, failure, and disposal lifecycle values without decoding
   binary output in JavaScript.
4. Both boundaries preserve bounded attempts, oscillation detection, resource
   resumption, and atomic stable publication.

The exit criterion is satisfied: label/reference and table-of-contents fixtures
become stable after bounded off-hot-path passes without delaying the
provisional result.
**Implemented:** hermetic closed source-only cases under
`tests/corpus/stabilization` cover primitive generated macros plus `\label`,
`\ref`, `\pageref`, and `\tableofcontents`;
native and wasm-bindgen tests compare exact stabilized artifacts and fixed-point
failure categories.

### Phase 3: paragraph replay deletion

The retained paragraph transaction subsystem was removed. Invalidated work now
restarts at an accepted command boundary or `JobStart` and executes normally.
Historical pre-deletion measurements remain in
[`paragraph_replay_deletion_baseline.md`](paragraph_replay_deletion_baseline.md).

## Required verification

The implementation gate must include:

- primitive TeX fixtures that read and rewrite a generated macro file;
- LaTeX `\label`/`\ref`, `\pageref`, and `\tableofcontents` fixtures;
- positive and negative probe transitions;
- unchanged generated bytes, changed unused output, and changed consumed input;
- cold versus incremental and cold versus stabilized byte identity;
- candidate failure, cancellation, resource suspension, pass-limit, and
  oscillation rollback;
- native and WebAssembly representation parity; and
- balanced optimized restart performance measurements.

Use focused `cargo test -q --tests` invocations while implementing, then the
repository's normal static and correctness gates. Fixture regeneration, when
needed, goes only through `scripts/regen-fixtures.sh`.

## Non-goals

- promising independent page compilation;
- inferring convergence from line count or pagination alone;
- parsing aux files into a label-only semantic model;
- changing TeX's synchronous file-open semantics; or
- exposing internal stabilization passes as editor revisions.
