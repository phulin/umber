# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes native expanded terminal words from `tex-command` and applies unexpandable TeX semantics to `tex-state::Universe`. `MainControl` is the only command-processing front. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal and vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits diagnostics through the state/world boundary.

Command operands are scanned by `tex-command` into typed request and result values. Execution code applies completed values through scoped reborrows of the actual retained processor owners, or after that borrow ends at a genuine World output boundary; it must not add another source cursor, raw-token delivery loop, scanner facade, or command compatibility adapter.

Shared terminal entry owns pending scanner reports and noncharacter character
flushes. Ordinary group open/close must not repeat this work or construct a
native-group preparation/result wrapper. Native delivery without an
exceptional record performs no observation check at continuation.

Group close drives the same `CommandProcessor::end_group` native stack walk in
the sole interpreter. That actual owner prints the cross-file warning before
restoration, renders saved values while their owners are live, admits aftergroup
words directly, and traces leaving through the restored live parameter. The
executor receives only completion or failure; no full group frame, restoration
receipt, token result vector or finishing stage crosses this seam. Ordinary,
observed, diagnostic and nested commands use it alike.

Fixed-size page publications pass their actual node arrays directly into the
admitted state owner. Box, math, discretionary and indentation wrappers must
not allocate a vector merely to publish one already-complete node.

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
- `src/main_control/tests/box_allocations.rs`: explicit profiling test of
  warmed empty and small-body box publication, register replacement and actual
  arena retirement without transient heap metadata or node vectors.

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
- `src/main_control/execution_state.rs`: the actual execution, mode, box,
  alignment, math, output and driver owner, disjoint from command input and
  processor loans. Main control owns `CommandState` directly; no interpreter
  wrapper or temporary replacement owner remains. Constructors initialize each
  actual owner once, including checkpoint forks. The execution owner implements
  live host fact queries; component-only cold loans share those exact queries.
- `src/main_control/interpreter/`: sole replacement core under construction.
  `delimiters.rs` scans actual delimiters and unsaves math-left groups
  through the live processor. Right boundaries append into the actual closed
  math tail without publishing an immediately consumed one-node sequence.
  `accent.rs` scans the actual accent and runs permitted assignments in the
  same processor before its base character, retaining the real accent font
  and metrics across those assignments. The reconstruction loop and accent
  request variants are deleted. `mod.rs` keeps the actual processor live across canonical expanded fetching
  and raw horizontal lookahead, applies the real host budget and publishes
  demanded scanner reports before mutations. `assignments.rs` scans and applies concrete register, parameter, macro and
  let mutations without ending the processor loan. `prefixes.rs` owns only
  TeX's actual prefix bits and the synchronous prefixed-command scan.
  `material.rs` scans and appends glue, kerns and penalties directly to live lists.
  `discretionary.rs` closes all three live break parts through native unsave;
  actual completed spans occupy three fixed slots, with no per-break vector
  or discarded scanner-result wrapper.
  `math.rs` appends actual styles/math characters, reserves noad nuclei before
  field scanning and transfers live braced fields and all four math-choice branches through native groups. The former math command adapters and nested math-choice execution loop are deleted.
  `recovery.rs` backs native spelling through canonical input, starts paragraphs
  before everypar replay, and inserts demanded math-shift recovery. `dispatch.rs` owns the single
  terminal mode/meaning switch; no native eligibility path or ordinary command
  product precedes it. `characters.rs`, `codes.rs`, `groups.rs`, `publication.rs` retain actual
  character admission, native selectors, unsave and final token publication.
  `glue_identity.rs` owns TeX pointer reassignment; `driver.rs` owns actual
  projection/high-water observations and admits the actual processor once for
  each interpreter invocation. Ordinary driver entry has no command slot;
  diagnostic preflight dispatches only its genuinely already-fetched record. Every selected terminal enters this native switch, including alignment
  recovery and endv. There is no selected-command fallback.
  Terminal dispatch receives the actual scalar key from expansion without an optional wrapper or fallback font selector. Native main-control, character, accent and alignment continuations use that key directly; codes.rs contains only compile-time selectors. Diagnostic preflight derives its key from the genuinely already-fetched command. Entry inspects the actual host facts directly; it lends
  mutable execution owners only when pending characters require a real flush.
  Fetch-role queries borrow actual host facts directly.
  Retained ordinary groups use the actual disjoint state/execution loans; saved records, aftergroup, tracing, math and recovery interrupt that span.
  Relax continuation reads the live World effect position and recoverable diagnostic demand
  without a mutable execution-owner callback.
- `src/main_control/interpreter/box_inputs.rs`: the one native `scan_box`
  operand selection and actual destinations. Direct register handoffs create
  no temporary construction mark; lastbox removal precedes destination
  interval creation. `leaders.rs` consumes actual following glue at the
  producing boundary. Old box, shift and leader request unions, the old
  box-builder/leader executor files, and pending-setbox/shipout mailboxes are
  deleted. Immediate stream output consumes scanner storage directly; there
  is no deferred extension union or immediate-print queue.
- `src/main_control/interpreter/math_shift.rs` and `display_math.rs`: native
  inline/display entry, font validation, equation-number completion, actual
  list packaging and display probes on the live processor. Font diagnostics
  publish before the synchronous report without re-admitting World. Native
  unsave restores parameters before resumed input. The former math-shift
  pairing packet and root math executors are deleted; the old alignment display-close helpers are deleted.
- `src/main_control/interpreter/alignment.rs`, `alignment_columns.rs` and
  `alignment_recovery.rs`: live preamble publication directly into the actual
  alignment, row and column selection, native entry unsave, noalign closure
  and canonical brace/delimiter repair. The old alignment executors,
  preamble/cell phase enum and pending scanner flags are deleted. Input
  interception and endv proof retain their command-core authority.
- The former `src/main_control/direct/` directory is deleted completely,
  including its dispatcher owner and processor reconstruction adapters. Do not
  restore it or add an equivalent compatibility dispatcher. Genuine opcode,
  character, group and publication primitives live under `interpreter/`.
- `src/main_control/semantic/`: concrete source-free typed semantic routines
  grouped by assignments, registers, fonts, text, paragraph/page state, groups,
  boxes/builders/leaders, alignment, diagnostics, streams, and
  PDF. Each routine borrows the actual admitted native owners; no operation
  union, readiness phase, context dereference facade, compatibility executor, or universal payload exists. Input stream scanners return a move-only final definition owner for read assignment; no attempt-definition promotion remains.
- `src/main_control/observations.rs`: demanded ordered mutation, effect and
  artifact publication after actual host work. Native input and semantic
  transitions publish evidence at their producers; there is no separate
  page-output observation owner or deferred application stage.
- `src/main_control/code_table.rs`: the shared TeX82 code-table value bounds,
  diagnostic, and zero-substitution rule. It returns a scanned value to the
  existing hot or cold assignment committer; it owns no command or table state.
- `src/main_control/host_boundaries.rs`: demanded evidence ownership,
  bounded host-driver accounting, halt, resource-need and diagnostic publication, ordered
  semantic/effect/artifact publication, and exact-cut main-source paragraph
  checkpoint intent. Shipout contributes only output-ledger evidence. Native
  scratch is reclaimed by consumers; there is no direct-operation cleanup.
- `src/main_control/interpreter/resources.rs`: typed provider fulfillment and
  PDF-image cache validation through the live context and host owners. Font
  and input-probe acquisition share its typed installer; image attributes
  render directly from attempt storage into their final PDF ledger entry.
  Same-run image provenance reads the actual World through the context.
- `src/main_control/root_source.rs`: immutable root-source registration,
  editor rebind, startup framing, and root completion policy on the same
  `MainControl` owner. See `docs/main_control_responsibilities.md`.
- `src/main_control/executor_facts.rs`: the borrow-scoped live mode and
  effective-tail provider, retaining either the actual mutable execution loan or
  a genuine immutable cold query loan, plus the stack-branded checked-save and delivery
  facts. A scanner or conditional invokes only the exact fact method it
  consumes; ordinary delivery writes no mode/tail cache and performs no
  effective-tail traversal. No live fact or provider survives a processor
  episode or enters a host resource-replay request.
- `src/main_control/definition_assignments.rs`: direct macro, let, catcode,
  and ordinary-group mutation helpers. Macro bodies are sealed once into their
  actual move-only publication owner. The macro arm keeps it across reporting
  and character-flush errors; assignment consumes it after acquiring the
  installed cell. Native read publication moves the same owner into its actual
  assignment arm. Let aliases the scanned key with no namespace promotion. Provisional
  shorthand tracing renders the old meaning before replacement releases it.
- `src/main_control/cold/`: genuine native alignment, PDF, grouping, and
  recovery helpers shared by the direct engine. `groups.rs` owns live group
  entry/unsave; `conditions.rs` owns partoken policy and final conditional
  reporting; `operand_values.rs` contains concrete arithmetic operands.
  `alignment.rs`, `pdf.rs`, and `support.rs` retain the actual builder, PDF,
  and source-free semantic owners. There is no cold operation scanner,
  promotion writer, phase wrapper, or cold dispatch authority.
- `src/canonical_step.rs`: shared bounded-step result protocol and the direct
  caller-owned fixed-chunk output ledger for exact-cut paragraph checkpoint
  publication, exact prior/current settlement, resource fulfillment,
  resource-restart accounting, cancellation, and borrowed terminal page capture.
- `src/engine_completion.rs`: handle-free terminal engine capture, aligned
  page/PDF projection, and non-clone effects-before-artifacts publication with
  exact suffix retry. Prepared openout replacements move complete artifact
  receipts and share their immutable bytes with the matching PDF page rows.
- `src/assignments/`: the `AssignmentCommitter` authority for scoped writes,
  e-TeX redundant-local decisions and tracing. Writes return only their observable-write decision; actual consumers construct final mutation records only when observation is demanded. It also owns
  primitive registration delegated from `tex-command`'s integrated catalogue.
- `src/assignments/tests.rs`: focused typed-owner controls for token assignment
  pre/post images across global replacement and local undo-backed writes.
- `src/box_runtime/`: source-free box-register, material, packing, migration, horizontal contribution, shaping, spacing, indentation, whatsit, leader, and list-commit operations. `vsplit/tests.rs` covers TeX82 §976 shrink normalization and unchanged-source ownership.
- `src/box_runtime/hmode/tfm.rs`: the sole forward-only TFM pair loop. It
  streams settled glyphs and kerns directly to the final sink and retains only
  characters required by ligature instructions. `tfm/scratch.rs` owns that
  stack, provenance spans, and the demand-only flat completed suffix for
  hyphenation reconstitution; previous/next work graphs and tombstones are
  absent. `tfm/tests.rs` checks physical-boundary ownership. Nested branch
  loans preserve parent scratch and truncate child suffixes on every exit.
- `src/paragraph_end.rs` and `src/paragraph_end/`: typed paragraph completion,
  one-time admitted linear borrowed-range post-line inspection,
  consumed semantic-source windows for generated line ownership,
  append-interleaved hyphenation over the same operation-local compact root
  and coordinate-only direct chunk continuations, coalesced unchanged-source
  materialization, packing, migration,
  contribution, diagnostics, pretolerance memoization, and focused
  pointer/copy-accounting tests.
- `src/paragraph_end/hyphenation/prefix.rs` and its `tests.rs`: TeX82 §914
  preceding-glyph reconstruction for hyphenated words, including ligature
  source preservation and font-kern boundary controls.
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
  releases the exact page-arena closure; failed staging or commit halts the
  run, and checkpoint restore rewinds the original roots.
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
- `src/checkpoint.rs` and `src/checkpoint/tests.rs`: JobStart and main-source
  outer-paragraph boundaries, editor forks with post-restore root-source registration,
  move-only aggregate checkpoint restore, budgets, bounded command/mode roots,
  profiling-only aggregate lifecycle probes, and retained token/glue-root
  restoration coverage. Capture preflights command/runtime admission before
  publication; public named restore requires the output ledger and prepared
  application is infallible. Component-only fixture restoration is test-local.
- `src/main_control/tests/aggregate_restore.rs`: repeated full public output
  restore and resource replay after shipout/effects inside an interrupted group
  and box builder, with no abandoned aftergroup delivery or fuel refund.
- `src/dispatch.rs`: dispatch result, execution statistics, and prepared-page contract.
- `src/execution_receipt.rs` and `src/execution_receipt/tests.rs`: crate-private
  typed operation receipts assembled and consumed by the unified executor's
  optional append-bounded evidence publication seam. Every allocating category
  closes before operation commit.
- Character admission derives Unicode scripts only for actual OpenType
  shaping fonts. Pending-run constructors accept that already selected script;
  TFM runs use `Common` without per-character Unicode classification. Font
  changes retain their existing flush and checkpoint semantics.
- `src/mode.rs` and `src/mode/tests.rs`: mode nest, page-arena list roots with
  direct single-node tail construction and actual multi-node active builders,
  checked `PageListSpan` live roots carried only
  within their admitting `PageRegion`, an owned mode-list slot that downgrades
  restored roots, a move-only removal seam for generated paragraph and
  alignment source ownership, direct-value paragraph/alignment glue, copy-only
  pending-character provenance, and rootless retained summaries. Restart
  eligibility proves a sole empty outer vertical level, so a named boundary
  stores that fixed scalar level directly and candidate forks retain no shared
  mode tail or transient mode payload. Candidate accept/reject consume one
  explicit lifecycle capability; normal drop cannot choose rejection, and
  terminal MainControl parking (which is not resource-replay packaging)
  preserves the capability until Session settles the aggregate. The move-only mode succession receipt is composed with state
  only after all exact roots are consumed. Alignment brace depth belongs only
  to `tex-command`.
- `src/mode/nest.rs`: the actual current semantic level, bounded suspended
  owners, cold ordered traversal and full mode checkpoints. Direct mode/list
  access performs no top-row selection; the former live-level slice API and
  identity-enabling no-op are deleted.
- `src/job.rs` and `src/job_output.rs`: TeX job framing, terminal continuation, final cleanup, and lazy DVI/transcript output. See `docs/job_framing.md`.
- `src/page_builder.rs`, `src/splitting.rs`, `src/vertical.rs`, `src/packing_params.rs`, and `src/pack_report.rs`: page accounting, vertical splitting/contribution, packing snapshots, and box diagnostics. Packing reports consume only
  `PackDiagnosticContext` line numbers and output-routine state; they do not
  capture or render an input-stack error context.
- `src/retained_resource.rs` and `src/session_api.rs`: host resource contracts, retained fulfillment, execution budgets, cancellation, and interrupts. PDF image requests are defined by `tex-command` and passed through this host boundary directly.
- `src/retained_generation.rs`: Non-generic move-only external-store slot
  lease, universally generic admitted engine episodes, host-owned
  full-checkpoint replay for resource needs, the canonical packed reusable-row
  boundary lane whose cells pair detached evidence with an optional move-only
  checkpoint root, stale-safe private owner-relative keys, typed cross-owner
  release transactions, and exact transfer of the sole output pool between
  accepted/current sidecars. An attached checkpoint-control guard restores
  runtime and MainControl sidecars without allocation or panic during unwind;
  it is an ownership guard, not a packaged scanner or executor continuation.
- `src/typeset_context.rs`: crate-private pure-kernel trait adapter over one
  already-admitted `CommandContext`; it owns no state, owner, or arena root.
  Math font-parameter readers borrow actual live font storage for the kernel
  call rather than resolving font identity for each numbered parameter.
- `src/**/tests.rs` and crate-local `#[cfg(test)]` modules: active semantic,
  replay, diagnostic, state, and exact-operand regression coverage selected by
  the library test target.
- `src/main_control/tests.rs` shares private test fixtures and names the
  behavior modules in `src/main_control/tests/`: initialization and formats,
  assignments, tracing, diagnostics and recovery, alignment, resource replay,
  shipout, PDF commands, list material, shared box bodies, paragraph and
  math, execution lifecycle, episode observation, and observed/unobserved
  equivalence. These remain one
  library test target.
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

Native execution errors box missing-font, missing-image and World payloads only
when raised. Resource failures move the same command-owned need/cause packet
through executor and session; successful results construct no failure payload.

- `src/main_control/interpreter/dispatch.rs`: the sole terminal selector,
  including packed math style, defined-symbol and numeric math heads. Selected
  arms scan actual operands inside the retained processor. `trace.rs` produces
  command displays only when tracing demands them. `boxes.rs` closes live box,
  insertion and adjustment groups through native unsave. It also consumes
  box/copy/lastbox/vsplit results into their actual append, register or shipout
  destinations. Unboxing and saved discards call their concrete list kernels. `streams.rs` publishes
  retained deferred-write tokens and renders immediately consumed message and
  special text directly from scanner storage. `diagnostics.rs` completes demanded displays and their error dialogue in
  the live processor; deferred show-completion and outer-page-build fields
  are deleted. `pdf.rs` selects actual graphics/navigation/form kernels with live owner
  loans, preserving pre-operand PDF-output and mode checks. It scans concrete
  PDF font operands and renders immediately consumed
  font-map and ToUnicode text directly into final bytes. `fonts.rs` scans font operands
  and acquires metrics through the actual capabilities and provider loans;
  no separate font-resolution admission remains. `termination.rs` owns real
  residual-page ejection and final input retirement. `driver.rs` releases the
  processor only for actual host work, observations and terminal publication.

Mark class/text and deferred openout operands scan in their selected switch
arms through the retained processor. Mark publication lends only actual
context, modes, diagnostic effects, fuel and boxes to its semantic routine;
TeX82 §1101 performs no page build or page-output capture. Its whole-executor
application wrapper and the second-processor streams-scanner module are
deleted. Restricted horizontal mark migration keeps its existing page region.

Arithmetic runs in `interpreter/arithmetic.rs` on the actual reader loan:
its expanded target word feeds operand scanning and semantic application
without rebuilding a processor. Demanded reports and error dialogue precede
mutation or the next scan. There is no assignment-loan macro or discarded
original-value carrier.

Native calls consume standard `Result` from the direct scanner APIs. Retained
scalar unions, result projections and compatibility scanner entries are absent.
The operation-child result adapter is deleted; callers map actual command errors.

Let assignments keep their native meaning word through the redundant-local
comparison and central admitted state write. Only demanded mutation receipts
decode that word; no resolved-meaning assignment API remains.

Completed restricted scanner requests carry their actual value and required provenance, with no discarded original-value or recovery flags. Missing-delimiter status remains with its real diagnostic consumer.

Native terminal consumption also admits commands after text. Relax and group
exit use their real pending-character flush; assignments keep their own scan
and flush ordering. Actual text transitions return a semantic completion so
the retained processor refreshes its fetch role and mode without a command
record or a second delivery loop.

Math-font family assignments run in the native assignment arm on the retained
processor. The second math-family scanner module, application wrappers and
unused family request are deleted. The central font-support validation and
scoped math-font state write remain authoritative.

The native consumer is specialized at its actual entry: bulk text boundaries
flush and return their real completion; ordinary admitted terminals continue
without per-word text-owner queries. The existing reader admission guards
remain with their actual call sites; no cached eligibility state is added.

Horizontal letter/other/character-definition words also enter the glyph builder
through the native consumer. Scalar resident text shares that same admission
helper. Already-completed semantics preserve their live character-loop state
through retained continuation; fonts and groups leave the loop at their real
semantic entry. No second context admission is used to resume native text.

The actual native consumer owns expansion at character boundaries. Bulk text consumes packed macro/primitive operands through the shared command reader without a delivered command or a second loop; lexical command-core consumers keep raw boundary semantics.

Native math letters, other characters and character definitions use the retained processor, shared mathcode projection and existing native noad builder. Active mathcodes keep canonical expansion/backup semantics on that processor. Horizontal glyphs return their actual completion from their own arm; math glyphs continue main control, and native assignments perform no glyph-completion test.

Math font validation retains the actual first failure and renders input only on failure at its existing publication barrier. Math lowering borrows CommandState for synchronous undefined-family diagnostics; MathConversionErrorContext and eager diagnostic string clones are deleted. Display/equation packing uses the existing live ExecutionDiagnosticContext. No input mutation or borrowed diagnostic owner crosses lowering or its subsequent group/input transition.

`interpreter/dispatch.rs` keeps common character, space and relax terminals in
the native inlined callback. `interpreter/dispatch/cold.rs` owns uncommon native
operations on that same retained processor. This split creates no eligibility
protocol, command product, compatibility executor or second input loop. Native
relax flushes its actual pending text directly; its semantic wrapper is deleted.

`interpreter/trace.rs` prints demanded native command traces. It does not
prepare per-fetch prefixes. `executor_facts.rs` lends the static mode name at
an actual print and updates the existing shown-mode owner immediately; cold
shipout processors borrow that same owner directly. Deferred write expansion
uses TeX82 §1370's no-mode setting at this producer, with no returned trace
flag or subsequent shown-mode repair.

Native continuation keeps its operation count locally and publishes it once
at the actual invocation return, including failures. The shared terminal entry
owns demanded reporting, character-run exit and tracing. An ordinary relax
then goes directly to continuation without constructing a successful
MainControlStep result; other semantic arms return their actual step through
the same native switch. No additional command eligibility front is introduced.

Page-material and page-region work-count assertions require `profiling` and
run through `scripts/check-tools.sh profiling-arena-tests`. Routine tests
retain shape, stable-address, stale-root and checkpoint/replay assertions.

- `src/main_control/interpreter/terminal.rs`: the actual synchronous interpreter consumer, owned operation-budget scalar and clean no-op/group admission. It owns no input or execution state and retains no meaning cache; input transitions and full terminal entry invalidate the local no-op fact.

Resident horizontal characters execute the canonical glyph append with the existing
CommandContext, fuel, diagnostic effects and execution owner while the sole command
reader keeps its stored-input span. Raw horizontal lookahead retains its TeX fetch
role and suppresses character command tracing. Noncharacters release the span before
expanded interpretation. Errors freeze their diagnostic input context only after
the reader loan ends; resource replay retains the existing full-checkpoint unwind.

The native terminal consumer fixes its TeX fetch role with a const parameter at
reader entry. It stores no duplicate lookahead flag, and clean no-op loops contain
no glyph-role branch. Raw horizontal lookahead alone requests the span's macro
invocation origin; other reader roles omit that unused ancestry scan.

Horizontal glyph append returns its actual classic-metric next-fetch decision.
Existence, shaping and false-boundary tests share one immutable LoadedFont loan;
source-character suffixes retain that payload across their read-only admission.
The forwarding glyph entry and OpenTypeShapingScratch compatibility alias are
deleted; mode owners use HorizontalFlushScratch directly.
