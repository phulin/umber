# tex-exec Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's stomach: main-control dispatch, modes, assignments, box-building side effects, and execution-time diagnostics.

## Crate Role

`tex-exec` consumes completed commands from `tex-command` and applies unexpandable TeX semantics to `tex-state::Universe`. `MainControl` is the only command-processing front. It installs and dispatches unexpandable primitives, manages the mode nest, performs assignments and grouping-sensitive state changes, builds horizontal and vertical material, invokes pure typesetting kernels, lowers shipped pages into `tex-out` artifacts, and emits diagnostics through the state/world boundary.

Command operands are scanned by `tex-command` into typed request and result values. Execution code applies completed values through scoped reborrows of the actual retained processor owners, or after that borrow ends at a genuine mutable-mode or host boundary; it must not add another source cursor, raw-token delivery loop, scanner facade, or command compatibility adapter.

Group close drives the same `CommandProcessor::end_group` native stack walk in the sole direct dispatcher. Its diagnostic callback renders each restoration while the saved owners are live; aftergroup words enter final input during unsave. There is no restoration receipt, token result vector, or delayed aftergroup scheduling helper. Ordinary, observed, diagnostic, and nested commands use that same group seam.

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
- `src/main_control/direct/`: the single packed mode/command/prefix switch in
  `dispatch.rs` and the reader-owned `native_assignments.rs` assignment switch, with synchronous `scan_*` routines and concrete `apply_*`
  methods organized by native command responsibility. `scan_macros.rs` releases
  actual scanner loans at cold owner and publication boundaries. The native
  fetch/reswitch loop borrows the complete actual execution owner, refreshing its mode for each command and querying box/trace/pointer state at the consuming arm. It retains its actual processor and context through RELAX, ordinary groups,
  integer/dimension/glue/token registers and parameters, arithmetic, definitions, let assignments, raw aftergroup/afterassignment operands, kerns, horizontal/vertical glue and setlanguage, consuming scanner products at their actual destination and respecting the caller's remaining budget. Font selection, font dimensions and character controls, generated fonts, hyphenation data, character-code banks and PDF font controls scan and mutate in their selected arm through that same processor. Their replaced scanner/application adapters are deleted; source-free helpers borrow actual context, effects, command state for recovery and demanded mutation receipts. Character and register shorthands, catcodes, paragraph shapes and penalty arrays also scan and apply through that same loan; their command-machine and second-scanner wrappers are deleted. Array assignment decodes old values only for demanded tracing and streams durable words without a temporary encoded-word vector. External font loading retains its actual resource resolution boundary. Horizontal glue and setlanguage lend their actual context, modes, effects and fuel directly; their former whole-executor adapters are deleted. An existing pending
  character buffer is flushed through the actual mutable mode loan. Source-character admission borrows execution through that same processor; its scalar boundary keeps the existing destination and cursor, without opening a temporary forbidden-facts processor. Actual diagnostic,
  observer/tracked, page/output, afterassignment and budget demands release the
  loan at the actual owner boundary; no command settlement tail exists. `scan_arithmetic.rs` scans concrete
  the final typed operand in that same processor; target selection and invalid-target
  diagnostics remain in the actual switch arm. Scalar scans consume typed values
  without generic frame publication/projection. Fetch/reswitch borrows the one
  initialized command slot at remaining semantic ownership boundaries. Ordinary
  relax words execute through the executor-owned terminal consumer inside the
  canonical reader; substantive groups, fonts and assignments run through
  one outlined semantic handler to bound the per-word reader code size; the old post-fetch relax continuation is deleted. The
  consumer borrows actual driver counters and preserves pre-fetched raw
  character lookahead. Remaining commands clear their tag at successful continuation and
  copying only values needed across real input overwrites
  and cold ownership boundaries. Native observation receipts have one demanded
  owner per processor loan; ordinary commands do not create/drop absent receipts. Register scans likewise stay in their native
  switch arms. These paths own no operation value or continuation. `characters.rs` consumes borrowed
  source spans in the existing horizontal loop. `publication.rs` validates and
  reserves the complete scanner token root set in the already-admitted context before streaming final native
  token owners and committing token assignments. Scalar and glue scans execute directly in their selected switch arm; no separate scanner admission or assignment application wrapper remains for these state-only writes. `command_codes.rs` derives switch keys from packed meanings;
  Native §1090 vertical cases use those keys directly in the switch.
  Brace ownership, math-family selection and vertical paragraph entry are actual packed switch arms; unrelated commands perform none of their owner tests.
  The startup prologue admits once; scalar page/save queries borrow their real
  stores without generation admission, and the page owner has one fire-up
  seam with demand-only named-list publication.
- `src/main_control/semantic/`: concrete source-free typed semantic routines
  grouped by assignments, registers, fonts, text, paragraph/page state, groups,
  boxes/builders/leaders, alignment/recovery, math, diagnostics, streams, and
  PDF. Each routine borrows the actual admitted native owners; no operation
  union, readiness phase, context dereference facade, compatibility executor, or universal payload exists. Input stream scanners return a move-only final definition owner for read assignment; no attempt-definition promotion remains.
- `src/main_control/semantic_owners.rs`: actual semantic-owner admission and
  ordered diagnostic, mutation, effect, artifact, and paragraph publication.
  Semantic helpers carry actual optional observer and resource-decline loans;
  the concrete operation observation buffer remains with its outer owner.
  Native context-consuming arms complete directly; observation publication is demanded by its actual instrumentation owner. Observation allocates evidence only when requested; deferred page-output
  evidence has an optional owner created by observed output routines. Provider loans return
  before propagation of semantic failure.
- `src/main_control/code_table.rs`: the shared TeX82 code-table value bounds,
  diagnostic, and zero-substitution rule. It returns a scanned value to the
  existing hot or cold assignment committer; it owns no command or table state.
- `src/main_control/leader_values.rs`: the concrete constructed/register
  leader payload that the leader-glue scanner consumes.
- `src/main_control/host_boundaries.rs`: demanded evidence ownership,
  bounded host-driver accounting, halt, resource-need and diagnostic publication, ordered
  semantic/effect/artifact publication, and exact-cut main-source paragraph
  checkpoint intent. Shipout contributes only output-ledger evidence. Native
  scratch is reclaimed by consumers; there is no direct-operation cleanup.
- `src/main_control/resources.rs`: typed host-resource resolution and
  installation for fonts, input streams, and PDF images. It borrows the same
  `MainControl` and `Universe` owners and preserves checkpoint-replay errors.
- `src/main_control/root_source.rs`: immutable root-source registration,
  editor rebind, startup framing, and root completion policy on the same
  `MainControl` owner. See `docs/main_control_responsibilities.md`.
- `src/main_control/discretionary.rs`: `\discretionary` group and part
  lifecycle plus the `\-` shorthand. It mutates the existing mode and node
  owners under the sole main-control episode; operand scanning remains in
  `tex-command` and host boundaries remain in `host_boundaries.rs`.
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
  reporting; `operand_values.rs` contains concrete arithmetic/math operands.
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
  typed operation receipts assembled, consumed, and reset in place by the
  unified executor's optional append-bounded evidence publication seam; every
  allocating category closes before operation commit while warmed category
  capacity remains with the operation owner.
- Character admission derives Unicode scripts only for actual OpenType
  shaping fonts. Pending-run constructors accept that already selected script;
  TFM runs use `Common` without per-character Unicode classification. Font
  changes retain their existing flush and checkpoint semantics.
- `src/mode.rs` and `src/mode/tests.rs`: mode nest, page-arena list roots with
  detached active builders, checked `PageListSpan` live roots carried only
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

- `src/main_control/direct/native.rs`: the main-control terminal consumer,
  including packed math style, defined-symbol and numeric math heads that append final nodes directly.
  General math dispatch gates its mode once, narrows the actual primitive once and
  selects scripts by packed category; no ScannedMathScript wrapper remains. The
  retained observed dispatcher shares that append operation; no style scanner
  request or duplicate style enum remains. The consumer continues
  borrowing actual driver operation counts and diagnostic origins. Ordinary
  relax, font selections and simple/semi-simple group words continue inside the canonical
  expanded reader. Entry and unsave share the actual group helpers; group exit
  rechecks restored tracing parameters. Only actual reporting/failure results
  are returned, without command records. Tracing, leaders,
  character lookahead and flushing retain their actual boundaries. The last
  native operation returns its actual budget-cut result without command construction.

Mark class/text and deferred openout operands scan in their selected switch
arms through the retained processor. Mark publication lends only actual
context, modes, diagnostic effects, fuel and boxes to its semantic routine;
TeX82 §1101 performs no page build or page-output capture. Its whole-executor
application wrapper and the second-processor streams-scanner module are
deleted. Restricted horizontal mark migration keeps its existing page region.

Arithmetic runs in `direct/scan_arithmetic.rs` on the actual reader loan: its
expanded target word feeds operand scanning and semantic application directly.
The shared `with_native_assignment!` seam renders demanded scan reports before
mutation. The old invalid-target executor adapter is deleted; its diagnostic
borrows actual command and effect owners without an operation request.

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
