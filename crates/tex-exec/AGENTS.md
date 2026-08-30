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
- `src/main_control.rs`: generation-typed sole production interpreter and
  execution driver. It owns the authoritative `MainControl` state, advance and
  direct-episode loops, explicit owner loans and returns, semantic application,
  complete-job versus fragment completion, and terminal parking. Ownership
  transitions are direct calls into the private sibling modules below; none is
  a facade or second command front.
- `src/main_control/delivery.rs`: same-borrow raw fetch, classification,
  expansion, alignment interception, general operand scanning, preflight phase
  transitions, and typed retry reconstruction into the resident operation
  destinations.
- `src/main_control/operation_frame.rs`: the singular stationary
  `OperationFrame`, compact payload and phase tags, adjacent caller-owned typed
  cold slot, and move-only suspension carriers.
- `src/main_control/settlement.rs`: direct-operation evidence ownership,
  begin/commit/rollback, resource and diagnostic retry settlement, and ordered
  semantic/effect/artifact/boundary publication.
- `src/main_control/executor_facts.rs`: the one stack-branded stationary mode,
  effective-tail, transaction, checked-save, and delivery preparation. The
  caller-owned preparation slot is filled and drained fieldwise across
  admitted delivery, availability checks, scanning, and application; no
  by-value preflight aggregate crosses those stages. Live executor owners fill
  or drain individual fields through borrow-scoped views; no admitted context
  or preparation survives the operation loan.
- The `OperationFrame` owns the admitted current command, parked expansion,
  scalar phase, delivery cursor, scanner child, partial direct scan, and one
  mutually exclusive compact operation payload in its own fields. The payload
  owns hot operands directly and names one adjacent caller-owned typed cold
  slot only for uncommon leaves. Scanning installs either leaf once; a compact
  `ScannedOperation` tag selects the next phase, preparation changes only
  attempt-root fields to prepared-root fields, and application consumes
  semantic leaves through a mutable borrow before clearing the slot. The cold
  slot moves beside the frame only through a genuine typed suspension;
  ordinary hot values never reserve its 264 bytes.
  Do not recreate a nested preflight-command or scanned-operation projection,
  or extract those fields merely to cross prepare, retry, rollback, or resume.
- Each topology-stable operation prepares executor host capabilities once from
  the authoritative live list. A stack-branded preparation carries the mode
  and shared effective-tail result plus the compact delivery/retry fields
  through scanning into application. Application drains those fields in the
  same caller slot; suspension and error re-entry recompute host facts while
  moving only the genuine typed continuation.
- `src/main_control/hot_apply.rs`: fused family-sized scan operands and direct
  in-place semantic handlers for the measured definition, let, catcode, and
  ordinary-group families. These commands bypass `ColdOperation`; the scanner
  writes the completed hot operation into the existing caller-owned
  `OperationFrame`, root preparation mutates that resident operation, and
  application consumes its fields without a second carrier. Definition, let,
  and catcode application, ordered evidence publication, and §1269
  `afterassignment` backup share one callback-scoped admitted context; a group
  transition ends admission before a possible page/host boundary. Detached
  mutation values and macro-body walks are demand-selected cold evidence.
- `src/main_control/cold/`: uncommon-command boundary against that same
  interpreter and semantic state. `operation.rs` owns the typed borrow-barrier
  values and the small attempt/prepared root fields that change domain in
  place; `scan.rs` owns uncommon operand collection; `apply.rs` mutably borrows
  the resident prepared operation and consumes only its semantic leaves;
  `alignment.rs`, `pdf.rs`, and `support.rs` isolate the corresponding complex
  families without introducing another executor.
- `src/canonical_step.rs`: shared bounded-step result protocol and the direct
  caller-owned fixed-chunk output ledger for coordinate-only checkpoint
  publication, exact prior/current settlement, resource fulfillment,
  suspension accounting, cancellation, and borrowed terminal page capture.
- `src/engine_completion.rs`: handle-free terminal engine capture, aligned
  page/PDF projection, and non-clone effects-before-artifacts publication with
  exact suffix retry.
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
- `src/paragraph_end.rs` and `src/paragraph_end/`: typed paragraph completion,
  linear borrowed-range post-line inspection, append-interleaved hyphenation
  over coordinate-only direct chunk continuations, coalesced unchanged-source
  materialization, packing, migration,
  contribution, diagnostics, pretolerance memoization, and focused
  pointer/copy-accounting tests.
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
- `src/align/`: source-free alignment completion, packaging, and width
  resolution over borrowed page-arena cursors. Setting retains unchanged
  source ranges and appends only replacement nodes through detached active
  builders; no production alignment path owns a `Vec<Node>`.
- `src/math/`: source-free math validation, coordinate-backed mlist lowering,
  and display packaging. Appendix G output streams directly into detached
  page-material builders; unchanged native leaves and display-prototype ranges
  retain their original arena addresses.
- `src/math/display/prototype.rs`: e-TeX saved display-line prototype and
  directed `app_display` list replacement.
- `src/math/display/tests.rs`: focused display-prototype reuse and
  no-prototype packing coverage.
- `src/checkpoint.rs` and `src/checkpoint/tests.rs`: command-only named
  boundaries, editor forks with post-restore root-source registration,
  move-only aggregate checkpoint restore, budgets, bounded command/mode roots,
  profiling-only aggregate lifecycle probes, and retained token/glue-root
  restoration coverage.
- `src/dispatch.rs`: dispatch result, execution statistics, and prepared-page contract.
- `src/execution_receipt.rs` and `src/execution_receipt/tests.rs`: crate-private
  typed operation receipts assembled, consumed, and reset in place by the
  unified executor's optional append-bounded evidence publication seam; every
  allocating category closes before operation commit while warmed category
  capacity remains with the operation owner.
- `src/mode.rs` and `src/mode/`: mode nest, page-arena list roots with detached
  active builders, checked `PageListSpan` live roots carried only within their
  admitting `PageRegion`, direct-value paragraph/alignment glue, copy-only
  pending-character provenance, rootless retained summaries, and the
  same-region operation-local rollback journal. Restart
  eligibility proves a sole empty outer vertical level, so a named boundary
  stores that fixed scalar level directly and candidate forks retain no shared
  mode tail or transient mode payload. Candidate accept/reject consume one
  explicit lifecycle capability; normal drop cannot choose rejection, and
  terminal MainControl parking preserves the capability until Session settles
  the aggregate. The move-only mode succession receipt is composed with state
  only after all exact roots are consumed. Alignment brace depth belongs only
  to `tex-command`.
- `src/job.rs` and `src/job_output.rs`: TeX job framing, terminal continuation, final cleanup, and lazy DVI/transcript output. See `docs/job_framing.md`.
- `src/page_builder.rs`, `src/splitting.rs`, `src/vertical.rs`, `src/packing_params.rs`, and `src/pack_report.rs`: page accounting, vertical splitting/contribution, packing snapshots, and box diagnostics.
- `src/host_api.rs`, `src/retained_resource.rs`, and `src/session_api.rs`: host resource contracts, retained fulfillment, execution budgets, cancellation, and interrupts.
- `src/interpreter.rs`: session-lived canonical command-state ownership,
  generation-typed processor facades borrowing one stable call-local admitted
  context in place, and assertion-bearing interpreter lifecycle accounting
  across semantic and host barriers.
- `src/retained_generation.rs`: Non-generic move-only external-store slot
  lease, universally generic admitted engine episodes, one singular typed
  same-thread suspension seam for non-atomic semantic owners, the canonical
  packed reusable-row boundary lane whose cells pair detached evidence with an
  optional move-only checkpoint root, stale-safe private owner-relative keys,
  typed cross-owner release transactions, and exact transfer of the sole output
  pool between accepted/current sidecars. An attached checkpoint-control guard
  restores runtime and MainControl sidecars without allocation or panic during
  unwind so the outer aggregate rejection can always reach every owner.
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
- `tests/support/mod.rs`: generation-scoped HRTB fresh and Plain fixtures shared by
  public-boundary integration tests; branded engine ids never escape callbacks.

## Validation

Run `cargo test -q --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration or corpus checks. Run the whole routine suite with `cargo test -q --tests`, then use `scripts/check.sh` for dprint, Biome, rustfmt, and clippy.

Working a canonical-command semantic or DVI divergence that touches main-control dispatch requires reading [Canonical Divergence Working Contract](../../docs/canonical_divergence_workflow.md) first.
