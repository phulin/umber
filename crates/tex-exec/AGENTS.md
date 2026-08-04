# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes completed commands from `tex-command` and applies unexpandable TeX semantics to `tex-state::Universe`. `MainControl` is the only command-processing front. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal and vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits diagnostics through the state/world boundary.

Command operands are scanned by `tex-command` into typed request and result values. Execution code must apply those completed values after the processor borrow ends; it must not add another source cursor, raw-token delivery loop, scanner facade, or command compatibility adapter.

## Boundaries

- Do not read raw input directly. All execution consumes `tex-command` delivery.
- Do not bypass `Universe` or expose raw substores, checkpoint internals, or handle constructors.
- Keep pure list algorithms in `tex-typeset`, immutable font parsing in `tex-fonts`, artifact serialization in `tex-out`, and file, clock, and random effects behind `World`.
- Preserve the mode boundary: stomach-side code owns baseline/interline side effects and list contribution, while pure packing and line-breaking routines stay side-effect free.
- Retired lexer, expander, executor, assignment-scanner, alignment-scanner, math-scanner, replay, and diagnostic fronts must remain absent from both compiled and dormant source.

## File Map

- `Cargo.toml`: execution-layer dependencies and workspace lints; neither normal nor development dependencies include a retired command crate.
- `src/lib.rs`: public surface and module wiring. Do not add a second command front or a crate-wide dead-code allowance.
- `src/main_control.rs`: sole production command delivery and execution driver, including complete-job versus fragment root completion and bounded terminal-input continuation.
- `src/assignments/`: unexpandable primitive registration, assignment identity, tracing, and typed state writes.
- `src/box_runtime/`: source-free box-register, material, packing, migration, horizontal contribution, shaping, spacing, indentation, whatsit, leader, and list-commit operations.
- `src/paragraph_end.rs` and `src/paragraph_end/`: typed paragraph completion, hyphenation, line materialization, packing, migration, contribution, diagnostics, and pretolerance memoization.
- `src/paragraph_memo.rs`: source-free paragraph dependency validation, replay, and provenance recipes.
- `src/page_output.rs`: input-free page-output selection, `\box255` packaging, insertion distribution, held-over material, page marks, diagnostics, and final `\end` state.
- `src/shipout.rs` and `src/shipout/`: typed page/PDF-form staging, direct artifact and DVI emission, normalization, lowering, replay hosts, transactions, and publication.
- `src/diagnostics.rs`, `src/error.rs`, and `src/error_report.rs`: canonical error identity, provenance, rendering, recovery reporting, and fatal propagation. `ExecError::Fatal` is TeX82 §81's non-local exit and only main control may catch it.
- `src/align/`: source-free alignment completion, packaging, and width resolution.
- `src/math/`: source-free math validation, mlist lowering, and display packaging.
- `src/checkpoint.rs`: command-only named boundaries, editor forks, aggregate checkpoint restore, budgets, and rooted mode summaries.
- `src/dispatch.rs`: dispatch result, execution statistics, and prepared-page contract.
- `src/mode.rs` and `src/mode/`: mode nest, list metadata, pending horizontal characters, summaries, and rollback journal. Alignment brace depth belongs only to `tex-command`.
- `src/job.rs` and `src/job_output.rs`: TeX job framing, terminal continuation, final cleanup, and lazy DVI/transcript output. See `docs/job_framing.md`.
- `src/page_builder.rs`, `src/splitting.rs`, `src/vertical.rs`, `src/packing_params.rs`, and `src/pack_report.rs`: page accounting, vertical splitting/contribution, packing snapshots, and box diagnostics.
- `src/host_api.rs`, `src/retained_resource.rs`, and `src/session_api.rs`: host resource contracts, retained fulfillment, execution budgets, cancellation, and interrupts.
- `tests/it.rs`: public-boundary, architecture, and compile-fail coverage, including the one-command-front source audit.

## Validation

Run `cargo test -q --tests -p tex-exec` after local changes. For CLI-visible behavior or shipout effects, also run the relevant `umber` integration or corpus checks. Run the whole routine suite with `cargo test -q --tests`, then use `scripts/check.sh` for dprint, Biome, rustfmt, and clippy.

Working a canonical-command semantic or DVI divergence that touches main-control dispatch requires reading [Canonical Divergence Working Contract](../../docs/canonical_divergence_workflow.md) first.
