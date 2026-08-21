# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes completed commands from `tex-command` and applies unexpandable TeX semantics to `tex-state::Universe`. `MainControl` is the only command-processing front. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal and vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits diagnostics through the state/world boundary.

Command operands are scanned by `tex-command` into typed request and result values. Execution code must apply those completed values after the processor borrow ends; it must not add another source cursor, raw-token delivery loop, scanner facade, or command compatibility adapter.

## Boundaries

- Do not read raw input directly. All execution consumes `tex-command` delivery.
- Commit completed Appendix G native-node transactions only at the executor's
  `commit_math_transaction` seam; do not recreate a parallel math node
  vocabulary or publish a partially lowered formula.
- Do not bypass `Universe` or expose raw substores, checkpoint internals, or handle constructors.
- Keep pure list algorithms in `tex-typeset`, immutable font parsing in `tex-fonts`, artifact serialization in `tex-out`, and file, clock, and random effects behind `World`.
- Preserve the mode boundary: stomach-side code owns baseline/interline side effects and list contribution, while pure packing and line-breaking routines stay side-effect free.
- Retired lexer, expander, executor, assignment-scanner, alignment-scanner, math-scanner, replay, and diagnostic fronts must remain absent from both compiled and dormant source.

## File Map

- `Cargo.toml`: execution-layer dependencies and workspace lints; neither normal nor development dependencies include a retired command crate.
- `src/lib.rs`: public surface and module wiring. Do not add a second command front or a crate-wide dead-code allowance.
- `src/episode.rs` and `src/episode/tests.rs`: typed semantic commit/barrier
  protocol, fixed-size operational counters, and focused
  rollback/publication perturbation tests. Internal group/rollback stops are
  retired. Coverage fallback is
  structurally absent: every retained root uses this same dispatcher loop.
- `src/main_control.rs`: generation-typed sole production command delivery and execution driver, including same-borrow delivery, expansion, tracing, and ranked ordinary scanning whose retry command is retained only at a typed resource barrier, trace-mode continuity across that borrow, command-owned undefined recovery before stomach dispatch, typed alignment, diagnostic-assignment, and nested-immediate continuations, snapshot-free direct episodes, complete-job versus fragment root completion, bounded terminal-input continuation, loaded-profile versus canonical engine-binary semantic selection, typed committed mutation/effect observations, attempt-owned macro operands across scanned assignment apply seams, the execution-owned memo capability, and direct tracked-region lifecycle and mode projection.
- `src/main_control/hot_apply.rs`: fused family-sized scan operands and direct
  in-place semantic handlers for the measured definition, let, catcode, and
  ordinary-group families. These commands bypass `ColdOperation` and
  `PreparedColdOperation` materialization; detached mutation values and macro-body
  walks are demand-selected cold evidence.
- `src/main_control/cold/`: uncommon-command boundary against that same
  interpreter and semantic state. `operation.rs` owns the typed borrow-barrier
  values; `scan.rs` owns uncommon operand collection; `apply.rs` owns their
  semantic dispatch; `alignment.rs`, `pdf.rs`, and `support.rs` isolate the
  corresponding complex families without introducing another executor.
- `src/canonical_step.rs`: shared bounded-step result protocol and output ledger for checkpoint publication, resource fulfillment, suspension accounting, and cancellation.
- `src/transaction_protocol.rs` and `src/transaction_protocol/tests.rs`:
  exhaustive canonical-command capability classification and mutation-free
  preflight. Compatibility assertions over exact owner/mark projections,
  admission variants, and preflight layout are retired; external rollback and
  parity tests are the migration authority.
- `src/assignments/`: the `AssignmentCommitter` authority for scoped writes,
  e-TeX redundant-local decisions, tracing, and typed mutation receipts, plus
  primitive registration delegated from `tex-command`'s integrated catalogue.
- `src/assignments/tests.rs`: focused typed-owner controls for token assignment
  pre/post images across global replacement and local undo-backed writes.
- `src/box_runtime/`: source-free box-register, material, packing, migration, horizontal contribution, shaping, spacing, indentation, whatsit, leader, and list-commit operations.
- `src/paragraph_end.rs` and `src/paragraph_end/`: typed paragraph completion, hyphenation, line materialization, packing, migration, contribution, diagnostics, and pretolerance memoization.
- `src/output_provenance.rs` and `src/output_provenance/tests.rs`: explicitly
  demand-selected, budgeted `ArtifactSourceResolver` inversion for copying
  live node origins into artifact-owned `ArtifactSourceRecipe` values, plus
  focused proof that unrequested provenance performs no resolution.
- `src/page_output.rs`: input-free page-output selection, `\box255` packaging, insertion distribution, held-over material, page marks, diagnostics, and final `\end` state.
- `src/shipout.rs` and `src/shipout/`: typed page/PDF-form staging, direct
  canonical artifact emission, normalization, lowering, fresh-shipout DVI
  plan co-emission in the same borrow-scoped traversal, canonical-byte replay
  for restored artifacts and leader pages, transactions, publication, and
  demand-selected render-origin column controls. Successful page detachment
  releases the exact page-arena closure; failed staging truncates only its
  speculative suffix so aggregate rollback can restore the original roots.
  Artifact effect sidecars use DTO-local `ArtifactEffectOrdinal` values;
  runtime `EffectPos` cursors remain inside the live World transaction. A
  successful page publication emits `ShipoutGeometry` through
  `ShipoutGeometrySink::committed_shipout_geometry`; the DTO contains only
  detached dimensions and counts, and live observer attribution is attached
  by main control after the callback crosses that explicit boundary.
- `src/diagnostics.rs`, `src/diagnostics/tests.rs`, `src/error.rs`, and
  `src/error_report.rs`: canonical
  error identity, provenance, rendering, recovery reporting, and fatal
  propagation. Context needed after a command borrow ends is passed as an
  owned rendered string, never recovered from a retained input/source handle.
  Page-activity diagnostics similarly detach their node, insertion, count, and
  dimension evidence through one short-lived `CommandContext` before rendering.
  Hot execution reporters accept an admitted `CommandContext` plus the detached
  `ExecutionDiagnosticContext`; focused tests pin routing and context rendering.
  `ExecError::Fatal` is TeX82 §81's non-local exit and only main control may
  catch it.
- `src/align/`: source-free alignment completion, packaging, and width resolution.
- `src/math/`: source-free math validation, mlist lowering, and display packaging.
- `src/math/display/prototype.rs`: e-TeX saved display-line prototype and
  directed `app_display` list replacement.
- `src/math/display/tests.rs`: focused display-prototype reuse and
  no-prototype packing coverage.
- `src/checkpoint.rs` and `src/checkpoint/tests.rs`: command-only named
  boundaries, editor forks with post-restore root-source registration,
  aggregate checkpoint restore, budgets, rooted command/mode summaries, and
  retained token/glue-root restoration coverage.
- `src/dispatch.rs`: dispatch result, execution statistics, and prepared-page contract.
- `src/execution_receipt.rs`: crate-private typed operation receipts assembled
  and consumed by the unified executor's optional append-bounded evidence
  publication seam; every allocating category closes before operation commit.
- `src/mode.rs` and `src/mode/`: mode nest, mutable native semantic/physical
  node buffers, barrier-published page-arena sidecars, direct-value
  paragraph/alignment glue, copy-only pending-character provenance, summaries,
  and rollback journal. Alignment brace depth belongs only to `tex-command`.
- `src/job.rs` and `src/job_output.rs`: TeX job framing, terminal continuation, final cleanup, and lazy DVI/transcript output. See `docs/job_framing.md`.
- `src/page_builder.rs`, `src/splitting.rs`, `src/vertical.rs`, `src/packing_params.rs`, and `src/pack_report.rs`: page accounting, vertical splitting/contribution, packing snapshots, and box diagnostics.
- `src/host_api.rs`, `src/retained_resource.rs`, and `src/session_api.rs`: host resource contracts, retained fulfillment, execution budgets, cancellation, and interrupts.
- `src/interpreter.rs`: session-lived canonical command-state ownership,
  generation-typed borrow-scoped processor facades, and assertion-bearing
  interpreter lifecycle accounting across semantic and host barriers.
- `src/typeset_context.rs`: crate-private pure-kernel trait adapter over one
  already-admitted `CommandContext`; it owns no state, owner, or arena root.
- `src/**/tests.rs` and crate-local `#[cfg(test)]` modules: active semantic,
  replay, diagnostic, state, and exact-operand regression coverage selected by
  the library test target.
- `src/main_control/tests/tracked_region_coverage.rs`: cross-layer tracked-region
  perturbation, omission detection, lifecycle cleanup, and recording parity
  proof for the supported ordinary-operation region.
- `tests/it.rs`: public-boundary, architecture, and compile-fail coverage, including the one-command-front source audit.
- `tests/fixture_parity.rs`: active TeX82 reference-observation corpus runner;
  executes every retained source and compares its explicit ordered terminal
  and log projection with the pinned reference.
- `tests/support.rs`: generation-scoped HRTB fresh and Plain fixtures shared by
  public-boundary integration tests; branded engine ids never escape callbacks.

## Validation

Run `cargo test -q --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration or corpus checks. Run the whole routine suite with `cargo test -q --tests`, then use `scripts/check.sh` for dprint, Biome, rustfmt, and clippy.

Working a canonical-command semantic or DVI divergence that touches main-control dispatch requires reading [Canonical Divergence Working Contract](../../docs/canonical_divergence_workflow.md) first.
