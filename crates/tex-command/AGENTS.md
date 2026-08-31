# tex-command Guidance

Read the repository-level `AGENTS.md` and `docs/tex_command_core.md` before
editing this crate.

## Crate Role

`tex-command` is the sole target owner of canonical source tokenization, input
levels, raw command delivery, expansion, macro calls, conditions, scanners,
alignment delivery, and static profile dispatch. It depends on `tex-state` and
must never depend on `tex-exec`; `tex-exec` consumes its completed
unexpandable commands.

`docs/expansion_memory_lifetimes.md` is the current implementation map and
retention audit for command-state generations, execution scratch, macro and
scanner nesting, input ownership, and suspension. Update it whenever this
crate changes an owner or exact reclamation point.

Host capabilities are borrow-scoped through `CommandHostContext` and must
never enter snapshots, formats, durable summaries, or owned command state.
Private state-machine modules must not be widened for compatibility with
retired input or expansion APIs.

Line acquisition is **not** a host capability. tex.web §31's `input_ln` is
`tex_state::CommandContext::input_ln`, alongside `\ifeof`'s §501
`read_stream_at_eof`, because a terminal line and a `\read` stream are
`Universe` state rather than a host acquisition keyed by request, and because
`tests/it/boundaries.rs` forbids `CommandHostContext` from every file on the
line path. It is the one line source for both of tex.web's callers -- §363's
`firm_up_the_line` and §483-§486's `\read` -- and §71's `prompt_input(#)`
prints its own prompt inside the acquisition, so the command core has no
print channel of its own outside the borrowed
`tex_state::CommandContext::begin_diagnostic` diagnostic channel
`\tracingifs` uses with the executor's operation-local detached-effect
collector (see `src/conditionals.rs`).

## File Map

- `Cargo.toml`: dependency-light crate manifest and boundary-test support.
- `src/lib.rs`: intentionally small public facade and private module tree.
- `src/attempt.rs`, `src/attempt/token_lane.rs`, and `src/attempt/tests.rs`:
  transitional scanner/operation scratch, including one attempt-owned
  fixed-chunk token lane, promotion, and suspension scope capabilities. Each
  scanner carries only a typed branch coordinate into that lane; finalization
  publishes the same words and truncation returns whole chunks. Macro
  invocation storage does not use this arena. `CommandState` owns the sole
  ordinary operation coordinate; the executor moves a coordinate-free opaque
  lifecycle edge. Only genuine suspension copies the coordinate into the
  pending package for owner-exact readmission.
- `src/execution_scratch.rs`: current-generation reusable execution scratch.
  The admitted macro frame's fixed nine-slot metadata owns the current argument
  cursor and first-scan facts while its resident writer settles each accepted
  delivered token once into one logically contiguous fixed-chunk LIFO lane.
  That same transition advances the cursor, brace depth, paragraph fact, and
  removable-outer-group aggregate; delimiter-prefix holdback retains the first
  classification until commit. Sealing changes only the frame role; retirement
  truncates to its absolute mark and returns suffix chunks to a reusable high
  water. A pending child can inherit a retiring parent's earlier reclaim mark,
  and its unpublished suffix may rebase only after the last active ancestor
  retires. Activations never own a heap buffer or arena scope. Suspended
  `scan_toks` owners occupy their own recyclable typed row lane because they
  carry a definition builder and attempt scope; every other typed continuation
  remains in the bounded heterogeneous continuation lane.
- `src/expansion_work.rs`, `src/expansion_work/control.rs`, and
  `src/expansion_work/tests.rs`: current-generation parked-expansion owner used
  only after a real immutable-resource suspension. Fixed command and typed
  control chunks keep stable addresses; one chunked name lane and complete
  logical marks support exact abort and reuse; move-only owner/serial keys
  reject foreign and ABA-stale access. Ordinary synchronous expansion stays in
  the caller-owned command slot. Structural `expandafter`, `csname`, name-lane,
  `scan_toks` wrapper, and PDF string-compare migrations remain separate
  reviewed cutovers.
- `src/host.rs`: borrow-scoped, nonserializable host-capability boundary.
- `src/profile.rs` and `src/profile/tests.rs`: public semantic character values,
  immutable command/character profiles, the distinct canonical compiled-engine
  semantics that survive loading an older format, capabilities, stable
  fingerprints, and focused value/identity tests.
- `src/state.rs`: exclusively mutable live aggregate command roots,
  persistent command state, retirement-owned executor replay-completion
  fences transferred directly to §325/§390 descendants, current-generation
  execution scratch, and direct semantic mutation.
  `src/state/attempt_transition.rs` owns direct-operation scope settlement and
  resource handoff, including the single state-owned ordinary attempt mark;
  `src/state/projection.rs` owns allocation-free retained-byte
  and dependency views; `src/state/executor_publication.rs` owns ordered
  executor-facing fact transfer. Each is an inherent implementation on the
  same authoritative `CommandState`; none adds a facade or state owner. A named checkpoint records
  bounded timeline coordinates without cloning the aggregate root; warmed
  command delivery performs no root admission. Resource
  continuations retain the exclusive
  current-generation lease, its same scratch lanes, typed ids, and integer
  resume cursors; resumption re-borrows dense state and cancellation drops the
  current candidate wholesale. These are process-local command state, never
  format or summary payload. The live `CommandState` also owns TeX82's three
  scalar stack maxima directly; they are operational session evidence outside
  snapshot roots and survive rollback without shared synchronization.
- `src/timeline.rs`: generation-owned reversible storage for the remaining
  copy-small command stacks. Immutable frame payloads are admitted once in
  dense rows; fixed-chunk journals retain first-touch inline execution state
  or generation-checked handles to displaced payloads. A row admitted or
  already replaced after the newest
  observable mark is overwritten directly on pop/push reuse only while its
  current occupant has no compact or stored inverse; partially captured reuse
  first journals a replacement. Only required marked or partially captured
  versions retain displaced payloads. Physical input, parameter, condition, group, aftergroup,
  and alignment rows survive a logical pop while a checkpoint can reach them.
  Fixed marks retain logical tops and packed journal positions; reject redoes
  the detached accepted suffix and accept releases its obsolete slab slots
  without a frame clone or third lineage.
- `src/scalar_journal.rs` and `src/scalar_journal/tests.rs`: reusable fixed-chunk
  bidirectional command-root journal, scalar marks, two-lineage suffix
  settlement, chunk reuse, and exact reverse-rollback/forward-redo tests.
- `src/command.rs`: public opaque, ephemeral current-command representation;
  resident input writes its packed spelling and resolved meaning together
  through one reborrow of the caller's `EmptyCommand`. The packed-token
  resolver receives that actual command as its in-place target, indexes the
  canonical dense eqtb row once, and writes the final static payload or sole
  macro owner without a projection carrier. Input returns only the scalar
  packed-resolution fact; the `CommandState` resident transition reclaims its
  original destination for suppression and required alignment treatment, and
  the processor receives only a copy-small
  ready/outer result. The executor then borrows the one caller-owned value through preflight and scanning,
  and moves it only into an actual retry or another semantic owner; it never
  enters a durable snapshot or format boundary.
- `src/direct_command_delivery.rs`: profiling-only mixed-meaning structural
  harness for direct dense-row writes into the caller-owned command. Its
  one/4,096-round gate counts row accesses, row decodes, final macro-owner
  acquisitions, whole-meaning/command copies, and warmed allocations without
  adding production state or an alternate delivery path.
- `src/processor/expand.rs`: canonical destination-directed raw/expanded
  delivery state machine and static primitive dispatch. Its const-specialized
  raw and expanded entries advance resident input into the caller's final slot,
  settle that delivery in place, and, when requested, inspect and expand the
  same resident command without a second command handoff. It also includes one
  main-control preflight entry that raw-fetches into the caller's destination,
  classifies that resident command once, publishes an ordinary unexpandable
  result directly, and continues through expansion in place only when needed.
  Undefined commands are reported and discarded before the following command
  returns to the executor. The ordinary driver owns one live
  current command and lends it through macro and ranked primitive expansion;
  it moves that value into continuation state only after a typed immutable-host
  suspension, and a resumed primitive retains §367's already-emitted trace
  instead of printing the command twice. The same rule covers the typed `\expandafter` operand and
  `\csname` accumulator frames.
- `src/processor/expand_structural.rs`, `src/processor/expand_input.rs`, and
  `src/processor/expand_convert.rs`: direct/static structural, source, and
  TeX/e-TeX conversion primitive families. They borrow the one processor and
  mutate its authoritative state in place; none owns a command facade or
  alternate delivery path.
- `src/processor/expand_pdf.rs`, `src/processor/expand_pdf_string.rs`, and
  `src/processor/expand_pdf_file.rs`: direct/static pdfTeX state/object,
  string/regular-expression, and immutable-file enquiry primitive families,
  including their exact typed suspension phases.
- `src/processor/expand_replay.rs` and `src/processor/expand_render.rs`:
  expansion-result insertion plus shared append-oriented TeX command and
  conversion rendering. These contain no expansion driver or semantic owner.
- `src/processor/mod.rs`: processor construction plus the opaque delivery
  cursor moved across an executor-owned typed resource continuation; it
  restores observation ordering but owns no command/input semantics. The
  processor's immediate-delivery freshness proof is only the episode-local
  sequence scalar; `CurrentCommand` remains the sole owner of the full
  input-level/position/sequence stamp.
  Every construction uses `CommandProcessor::new`, which takes the
  caller-owned admitted context, session-owned fuel, observer, and
  operation-local diagnostic-effects collector directly and constructs no
  temporary owned ledger or whole-context handoff.
- `src/error.rs`: command error and resource-need representation plus the
  shared dimension-scanner recovery diagnostic vocabulary consumed by legacy
  and canonical scanner paths.
- `src/fuel.rs` and `src/fuel/tests.rs`: checked finite command-work limits,
  one exact remaining-budget countdown, separately stored non-fuel work
  counters, terminal/publication-time consumed-fuel derivation, the
  constructor-free borrow-only `CommandFuel` capability used by leaf
  operations, and the top-level `CommandFuelLedger` session owner shared by
  every canonical processor episode. Fuel and work counters are operational
  evidence outside semantic state; rollback never refunds them.
- `src/fatal.rs` and `src/fatal/tests.rs`: TeX82 §93 `fatal_error`,
  §94 `overflow`, and §95 `confusion` as one shared irrecoverable-error
  value, its canonical observation record, and focused label tests. Every
  present and future `succumb` site raises this type; §81 `jump_out` is
  modelled by the executor latching it as the session's terminal state.
- `src/input/source.rs`, `src/input/source/tests.rs`: public host-neutral
  one-shot source-registration inputs and errors plus private immutable backing,
  source cursors, retained World modification metadata for typed file
  enquiries, and focused registration tests. It also owns
  `SourceNameClass`, TeX82 §303's partition of a source level's `name` into
  the terminal (`name=0`), input stream `name-1` (`1..=17`), e-TeX's
  `\scantokens` pseudo-file (`name=18` or `19`), and a text file (`name>19`).
  That is a third, independent classification: `InputReason` is
  a strict model of §307's `token_type` codes, which a source level does not
  have at all, and `RegisteredSourceKind` names how Umber acquired the bytes
  rather than which channel TeX reads them as. An open with no explicit class
  is §537 `start_input`'s file, because §537 is how TeX reaches every `\input`
  and the job's root file alike; only §331's terminal and §483's `\read`
  streams need `open_registered_source_as` (`umber2-johp.245`). It also owns
  `SourceRegistration::with_name` (§537's `a_make_name_string`). Named `File`
  levels are command-framed by default; the explicit
  `SourceFramingPolicy::ExternallyOwned` exception preserves an authored root's
  `File` identity while suppressing the transcript frame when its surrounding
  host wrapper owns it. File and traced-`\scantokens` pushes return their one
  call-local opening name to the processor that already owns a live
  `CommandContext`; source retirement similarly returns one close bit through
  `InputRetirement`. The processor prints each at the transition itself, with
  §362's `)` after `file_warning` and ahead of the next
  `check_outer_validity` diagnostic. Startup drivers render the registered
  root's existing source-owned name at their selector boundary. Command state
  owns no framing-event queue or snapshot cursor.
- `src/input/lines.rs`, `src/input/lines/tests.rs`: exact physical-line
  splitting, TeX line normalization, byte/scalar cursor and range accounting,
  and focused line-contract tests.
- `src/input/tokenizer.rs`, `src/input/tokenizer/tests.rs`: canonical
  token-at-a-time exact-byte and separately identified UnicodeExtended M/N/S
  tokenization, semantic control-sequence spelling, production projection of a
  borrowed untransformed control-word slice or owned superscript fallback into
  packed identity plus direct provenance, profile-specific superscript notation,
  invalid-character recovery steps, byte/scalar ranges, and focused conformance
  tests. A cursor with no loaded line returns `NeedLine`; physical acquisition
  belongs to the singular input-top owner and never receives a backing registry
  through the ordinary token path.
- `src/input/levels.rs`, `src/input/levels/tests.rs`: canonical fixed-width
  source/token cursors over one `PackedTokenSpanHandle` shape. Replay, macro
  replacement/argument, attempt, and durable sources adapt once at level
  creation; ordinary delivery writes through that lifetime tag into the
  caller's final `CurrentCommand` through a reference-only `EmptyCommand`
  reborrow, resolves the resident packed word directly, returns only the
  scalar packed-resolution fact,
  reuses the resolver's literal-catcode classification for brace treatment,
  and advances only the packed frame scalar. A
  source frame installs its external-source identity in the common packed
  frame, and replay/macro frames inherit that context at admission. Delivery
  consequently carries the active source to main control with one top-row read
  and no ancestry or source-slot lookup. Resident source tokens and literal
  macro arguments return directly; stored-token access projects its optional
  packed out-parameter slot while the word is resident, and only that branch
  enters parameter replay. A parameter candidate overwrites the same
  unresolved value before meaning resolution; there is no raw command envelope
  or second delivery slot. A
  source row contains only its common frame and one eight-byte ABA-checked key
  into the `InputStack`'s fixed reusable source-slot pages. The slot solely
  owns backing, replacement, `everyeof`, ancestry, source classification, and
  retirement state. Ordinary history stores
  only its copy-small lexer cursor; cold line, backing, and everyeof changes
  move prior owners into the same logical-stack journal. The slot's runtime-only
  incarnation never rolls back with its semantic `InputLevelId`, so stale or
  foreign compact and stored inverses fail before mutation. The module also owns cold backup source coordinates,
  explicit stored/transient/backed-up TeX82 cell ownership, exact LIFO segment
  reuse, and orthogonal delivery/retirement classifications. A source slot's
  optional `open_depths` owner is `\tracingnesting`'s own record. Nested source
  opening installs it before the frame becomes visible; retirement borrows it
  before pop and carries only copy-small boundary facts afterward. See
  `src/tracing_nesting.rs`.
- `src/input/history.rs` and `src/input/history/tests.rs`: the authoritative generation-tied `InputStack`.
  Stable source, stored-token, and macro-argument rows share one ordered
  `InputUndo` journal for first-touch advances, cold source-owner swaps,
  replacement/reuse, retirement, rollback/redo, candidate settlement, and
  prefix release. Repeated source-owner changes coalesce to the first inverse
  per checkpoint-visible row and interval; rows admitted in the interval need
  no inverse. Alternate owners live only in checked reusable slabs; there is no
  generic logical-stack adapter or second input representation.
  Typed cold source operations and one `CommandState`-owned resident front
  are the only mutable access. That front selects the semantic top once into
  a source, stored-token, or macro-argument view; each view borrows only its
  cursor and matching first-touch journal fields. The three branches share
  direct final-command writing and common post-borrow settlement, while
  preserving their distinct storage owners. Source, token-list, and macro-
  argument cursors therefore share a single top access, matching first-touch
  transition, direct final-command write, fuel/alignment/parameter settlement,
  restartable exhaustion pop, retirement observation/alignment/replay settlement,
  and diagnostic revision without a callback or returned exhaustion result.
  The resident transition continues immediately from the new top after an
  ordinary token or macro-argument pop, including a v-template that has
  crossed its retained `do_endv` boundary. The
  resident packed frame is read through only the required scalar fields and is
  never copied as a whole. No raw mutable top or mutable index survives.
  Focused gates prove exact coalescing and zero-allocation hot mutation. A resident source row
  structurally proves its source slot remains occupied, so ordinary source
  delivery borrows that slot by physical index without repeating the ABA
  generation check. Cold history, rollback, and detached coordinates continue
  to validate the complete slot key.
- `src/input/stack.rs`, `src/input/stack/tests.rs`: exact input retirement.
  Ordinary exhausted token and macro-argument rows are projected and popped
  from the already-selected resident coordinate, as is a v-template after
  successful `do_endv`; terminal token input, pre-`do_endv` retained
  v-templates, source EOF, recovery, and parameter results retain
  no command-slot borrow; an ordinary final result has already resolved
  meaning, applied one-delivery suppression, classified only required
  alignment work, and returned no command reference. The files also own the singular
  physical-line acquisition owner, the canonical frame-push transition and scalar maximum
  update, centralized replay-lane admission, retained v-template lifecycle,
  macro-activation cleanup, borrowed source-ancestry comparison with copy-only
  retirement facts, `param_start` parameter replay ownership, and
  trace-independence tests.
- `src/input/mod.rs` and `src/input/tests.rs`: tex.web §§310--318's live error-
  context traversal and omission matrix. The traversal selects the current,
  `\errorcontextlines`-budgeted, and bottom levels before pseudoprinting; it
  retains only one deferred bottom coordinate and never materializes omitted
  level strings.
- `src/processor/`: public borrow-only processor facade with const-specialized
  raw and expanded entries into one destination-directed state machine,
  expansion, scanner-status, and alignment orchestration. The fused
  destination pipeline advances resident input and inspects the command
  in place; its sole delivery settlement applies noexpand, outer
  validity, alignment classification, and observation after dense resolution
  has ended. The two specializations share canonical
  token-to-current-meaning delivery;
  creation permission exists only at source tokenization and is absent from
  delivery policy. The loops do not test a raw-versus-expanded mode on every
  token. Internal steps return a compact status or zero-sized failure marker;
  one request-local cold error slot carries a real error to the public boundary.
  Expanded delivery classifies each resolved meaning once as return, expand, or
  end-template handling.
  The facade also resumes an executor-retained settled delivery. Main-control
  preflight performs raw fetch, expansion classification, and any required
  expansion in one driver entry without backing up or redelivering the command.
  `status.rs` owns the one processor-level scanner episode mechanism for
  typed status entry, observation visibility, recovery re-entry, and complete
  prior-state restoration; scanner families do not open-code that lifecycle.
- `src/processor/next.rs`: public raw-delivery policy entry points plus
  demand-only resident-command observation after all dense borrows end. The
  fused state machine in `expand.rs` delegates cold source retirement, outer
  recovery, and alignment interception directly to their private semantic
  modules without introducing another command or input owner.
- `src/processor/end_input.rs`: physical-line acquisition, source exhaustion,
  `\everyeof`, final cleanup, terminal/v-template retirement, and explicit
  §§325/390 stack conservation. It is the cold processor retirement boundary;
  ordinary delivery retirement stays inside the resident input transition.
- `src/processor/backup.rs`, `src/processor/recovery.rs`, and
  `src/processor/outer_recovery.rs`: exact command backup/replay insertion,
  executor-facing recovery operations, and scanner-status outer/runaway
  interception respectively. They share the live processor directly and add
  no recovery queue, command slot, or runtime dispatch layer.
- `src/processor/alignment_interception.rs`: alignment-aware delivery policy,
  typed delimiter/v-template handoff, and active-cell input proofs over the
  canonical alignment state; it never reclassifies input in the executor.
- `src/processor/tests.rs`: tracked command-root publication and fail-closed
  unsupported-continuation coverage.
- `src/processor/alignment.rs`, `src/processor/alignment/tests.rs`: canonical
  alignment-delivery state and focused stack, brace-depth, template, and omit
  lifecycle tests.
- `src/processor/expand/tests.rs` and `src/processor/fixtures/`: focused
  expanded-delivery, primitive, suspension, allocation/layout unit tests and
  bounded source microfixtures.
- `src/scanners/`: private typed scanner family. `hyphenation.rs` owns TeX82
  §934/§960's `\hyphenation`/`\patterns` scans, which are `get_x_token`
  classification loops rather than `scan_toks` collections and so must never
  enter the `absorbing` scanner status or track a brace depth.
  `expression.rs` and `expression/tests.rs` own e-TeX 2.6's four typed
  expression scanners and two glue-level conversions, including their
  explicit parenthesis stack, arithmetic recovery, glue-order rules,
  observations, and checkpoint-retry tests.
  `structured.rs`'s `scan_accent_base` is deliberately a one-command step
  rather than a loop: TeX82 §1123 runs §1270's `do_assignments` between the
  accent code and §1124's base character, and executing an assignment is the
  executor's, not the scanner's. It replays only §1124's own `else
  back_input`, inside the delivery episode that fetched the command; a
  prefixed command is handed back still delivered, because §1270 executes it
  in place and a backup level here would deliver it twice.
  `scalar.rs` owns `back_input_unless_spacer`, TeX82's
  `if cur_cmd<>spacer then back_input`. §443's `⟨Scan an optional space⟩`,
  §444's `⟨Scan a numeric constant⟩`, and §452's `⟨Scan decimal fraction⟩`
  all end a numeric scan with that one rule, so every numeric scan routes its
  terminator through it rather than choosing per call site whether to absorb
  a space. Expandable `\number` and `\romannumeral` scans retain their leading
  sign/provenance, radix-tail accumulator, or completed §442 character code
  awaiting its expanded optional-space probe when delivery suspends on an
  immutable host request, so retry resumes the exact TeX82 §§440--445 token probe. The terminator test is
  on the command, so it is the category code and never
  the character: §207 makes `spacer` the command a category-10 character
  carries, and §349 is what normalizes such a character's `cur_chr` to a
  space inside §341's `get_next`. The same reusable ABA-tagged scalar lane owns
  optional-equals, fixed-inline keyword prefix, integer, dimension, glue,
  filename, internal-value, expression, and font-selector continuation state.
  Raw resource-capable scanners remain private; executor scalar phases write
  once into the reusable `ScalarScanFrame` owned by their `OperationFrame` and
  return only a compact status. Internal expansion, conditional, alignment,
  and structured parents move a suspended child into their exact typed phase.
  Success, resuspension, abort, and
  fallible parent-frame storage all close or reinstall that chain
  deepest-first. Do not add a root mailbox, caller-order result tape,
  destination inference/search, or command redispatch fallback.
  `filename/tests.rs` owns focused expanded filename scanning, termination,
  replay, and registered-source retry tests.
  `structured.rs`'s `scan_math_field_episode` is TeX82 §1151's `scan_math`,
  a classification and not an absorption: every scalar case ends holding one
  math code, so the field pushes no input level, backs up no token, and never
  redelivers the command that selected it, and `othercases` is the whole
  remaining vocabulary reaching §1153's `back_input; scan_left_brace`. A
  frozen-spelling replay level here delivers each field twice
  (`umber2-johp.265`).
  `restricted.rs` owns TeX82 §433-§437's five restricted integer classes as a
  single mechanism: every bounded scan in the crate selects a
  `RestrictedIntegerClass` instead of open-coding a range test, and the
  recover-to-zero belongs to the scan, never to the command consuming it.
- `src/primitives/`: private static primitive dispatch. The integrated
  catalogue spans `primitive_metadata.rs`'s exhaustive enum rows,
  `parameters.rs`'s parameter cells/defaults, and `generated.rs`'s exceptional
  meanings (`nullfont`, frozen `endwrite`, page quantities, and extension
  internals). `generated.rs` is the sole projection seam for stable operands,
  exact profile name sets, installation slices, observation identities,
  prefix policy, and deterministic documentation tables. `registry.rs`
  consumes those views for fresh INITEX and format restoration; it contains
  only store-local installation mechanics. `catalogue.rs` defines the
  behavior-free descriptor vocabulary and exhaustive validation. Execution
  dispatch remains handwritten.
  `prefixed.rs` owns TeX82 §209's `max_non_prefixed_command` partition -- the
  single test §1211's `prefixed_command`, §1270's `do_assignments`, and the
  `\global` prefix all make. It is narrower than "this command assigns
  something": `\begingroup`, `\endgroup`, `\aftergroup`, `\afterassignment`,
  `\openin`/`\closein`, and every `extension` primitive are excluded, so a
  caller that starts from a broader notion and subtracts them by hand is
  re-deriving this predicate one exception at a time.
- `src/macro_call.rs`, `src/macro_call/tests.rs`: private canonical scalar
  macro matcher, destination-directed construction into execution-scratch
  segments, stable sealed direct-slot descriptors, uniform one-scalar parameter
  replay, exact tail/nested retirement, and focused tests.
- `src/conditionals.rs`: private independent condition-stack machine; also
  renders e-TeX 2.6's `\tracingifs` `{...}` trace lines at conditional entry
  and at each `\or`/`\else`/`\fi` delimiter resolution, printed directly
  through `tex_state::CommandContext::begin_diagnostic` into the enclosing
  operation's detached-effect collector because tex.web's `show_cur_cmd_chr`
  fires from inside `conditional`/`pass_text` itself rather than through the
  executor.
- `src/tracing_nesting.rs`: renders e-TeX 2.6's `\tracingnesting`
  `file_warning` -- "Warning: end of file when ... is incomplete" for every
  group and conditional still open at a source level's natural EOF, compared
  against the depth `state.rs`'s `record_source_open_depths` recorded when
  that level opened. Called from `processor/end_input.rs`'s cold
  `retire_input_top` source branch. The
  source-only boundary probe checks the validated top row directly; ordinary
  token and macro retirement never searches the enclosing input stack. Prints
  through the ambient selector (`CommandContext::printer`), not
  `begin_diagnostic`'s `\tracingonline` redirect: unlike
  `\tracingassigns`/`\tracinggroups`/`\tracingifs`, `file_warning` is not
  `stat`-gated in `etex.ch`. The same module owns `if_warning` and the
  ordinary/semi-simple `group_warning` close path. `\scantokens` pseudo-files
  record the same opening depths as ordinary inputs. Remaining specialized
  group-close sites are tracked by `umber2-aqx9`.
- `src/token_collector.rs`: shared scanner-owned in-progress collector for
  macro arguments, token-list scans, and definition scans. It owns the one raw
  classification, brace/delimiter/parameter state, destination phase, and
  publication cursor while preserving activation-scratch versus
  attempt/durable destination lifetimes.
- `src/scan_toks.rs`, `src/scan_toks/tests.rs`: private canonical token-list
  scanner and focused parameter, collection, expansion, scanner-status, and
  recovery tests. A scanner owns no arena or scope. Temporary collection uses
  the scanner word/builder lanes. Macro `\def`/`\edef` and `read_toks`
  collect semantic words into one attempt-local recyclable definition builder;
  the builder's one non-atomic allocation becomes the final current-generation
  `DefinitionId`, so successful publication retains it without allocating or
  copying words. Generic cold-operation promotion borrows that exact builder in
  its attempt row, validates its original destination identity policy before
  publishing any batch row, then writes the durable owner directly into the
  checked resident operation field and releases scratch ownership only after
  success; failure needs no owner restoration. It constructs neither a
  builder batch nor an aggregate owner receipt, and never copies the parameter/
  replacement slices or reconstructs a second builder. Read setup and
  finalization share one cleanup transaction which
  restores `align_state` and scanner status and truncates the exact child scope
  on every error. General token-list scans append directly to independent
  branches in the attempt's shared fixed-chunk lane; they never own or recycle
  a per-scan word vector, and sealing is a metadata-only publication. Nested macros
  use separate macro frame/argument lanes, so push/pop never interleaves their
  scratch with scanner output. A suspended scan carries branded frame indices
  under the same exclusive current-generation lease. Scratch-frame insertion
  preflights allocation, capacity, and serial advance before moving that scan;
  failed insertion or a conflicting processor-baton publication restores the
  prior baton, aborts the nested child deepest-first, finishes scanner status,
  and reclaims both the scanner suffix and its parent-owned sink rows while
  retaining reusable lane and builder capacity. Its semantic
  `ScanToksMode` constructors are parsed once
  into a typed internal grammar, opener, expansion, warning owner,
  observation purpose, and status-visibility configuration. It also owns
  TeX82 §482's `read_toks`, which is deliberately
  _not_ a `scan_toks` mode: it collects whole lines rather than a
  brace-balanced group, holds `align_state` at `1000000` for its whole
  duration, and continues across a brace imbalance instead of ending at a
  closing brace. §486 does not balance a runaway `\read` by inventing
  braces; its whole recovery is `align_state:=1000000; limit:=0` plus the
  error, so the stored list keeps exactly the tokens the file supplied.
- `src/observation/`: private aggregate read observation. `mod.rs` owns the
  record union, engine-owned source identity and alignment nesting, and the
  exhaustive `Meaning`-level command classification;
  `primitive_identity.rs` and `variable_identity.rs` own the exhaustive
  primitive and eqtb-addressed variable identities beneath it. None of the
  three may reintroduce a catch-all: an unclassified meaning must be a build
  failure, never a plausible generic identity in a trace.
  `variable_identity.rs` also owns `parameter_mutation_key`, the single
  supported way to name a mutated named parameter. Umber's dense parameter
  bank slots are not tex.web's parameter codes, so an observation that
  formats a raw `IntParam`/`DimenParam`/`GlueParam`/`TokParam` slot reports
  one parameter's assignment under another's name (umber2-johp.134).
  `canonical_names.rs` generalizes that rule to the whole vocabulary: it is
  the only place any observation name is spelled, and it is re-exported as
  `tex_command::canonical_names` so producers in other crates and the
  differential tracer share one table rather than each keeping their own.
  See "Canonical observation vocabulary" below.
  Command identity is selected through the immutable `CommandProfile`:
  `variable_identity.rs` owns the TeX82/e-TeX 2.6/pdfTeX 1.40.29 eqtb and
  `last_item` layouts, while `primitive_identity.rs` owns profile-dependent
  conversion selectors. Never classify an observed command through a
  profile-free dialect approximation.
- `src/snapshot.rs`, `src/snapshot/boundary.rs`, and `src/snapshot/tests.rs`:
  generation-generic command
  snapshots and named summaries backed by generation-checked reusable frame
  pages plus packed scalar journals. One coarse generation owner retains the
  attempt mark and exact timeline-row identity; the copied snapshot cursor is
  only that row's serial, while the private row owns each real logical-stack
  rollback mark once. It contains no duplicated stack/queue lengths or
  aftergroup payload census.
  The private boundary module owns capture, validation, publication,
  fork/reject/accept, restore, and rollback over the substrate in `snapshot.rs`.
  Capture appends a move-only frame; aggregate release returns that frame row
  and every obsolete journal/logical-stack prefix chunk to their pools because
  JobStart is frozen outside the live owner. Retained-owner clone copies only
  scalar coordinates. Main control
  parks its exclusive physical command owner in the retained generation before
  candidate fork. Stable frame rows link accepted order directly; selecting a
  mark changes a fixed number of links to detach the later accepted rows and
  opens the only current suffix without a `VecDeque`, position search, or key
  copy. Reject rewinds current cells, transfers the whole candidate chain to
  the reusable-chain owner, redoes detached prior cells, and reattaches their
  row chain; accept retains candidate rows and transfers the detached prior
  chain. Settlement never visits a discarded row or manufactures a free-list
  key. Later publication takes one reusable row, drops its obsolete payload,
  and assigns the fresh incarnation that rejects its ABA-stale mark.
  Validation never mutates the runtime,
  aggregate command roots are not `Clone`, and capture requires quiescent
  execution scratch.
- `src/continuation.rs` and `src/continuation/`: handle-free command-summary
  and suspended-execution recipes, dense DTO-local indices, recursive schema
  validation and budgets, cold detachment construction, destination-stamped
  staging, atomic publication, and focused rejection/retry tests. The schema
  contains no runtime identity, owner, storage coordinate, or borrow.
- `tests/`: external dependency, visibility, and capability-boundary tests.
  Character/input integration coverage binds the exact shared-domain tokenizer
  to the pinned TeX82 fixture and compile-fail gates profile immutability.

## Canonical Observation Vocabulary

Every string an observation payload carries for a _concept_ -- a category
code, a character command, a scanner status, a glue order, a token's catcode
or spelling, a meaning's command name -- is spelled once, in
`src/observation/canonical_names.rs`, and nowhere else. Producers in
`tex-command` and `tex-exec` and the differential tracer all call it.
Scanner results use `ObservationValue`; producers must retain integer, scaled,
glue, name, and token-list domains instead of rendering values for a detached
consumer to parse. Mutations pair a typed `MutationTarget` with typed key and
value fields; effects pair `ObservationEffectKind` with separate channel and
value fields. Neither family may encode structure with prefixes, separators,
or numeric text for `tex-observe` to reverse.

Rules, all of them load-bearing (umber2-johp.141):

- **tex.web is the authority, never Umber's enum spellings.** §207 fixes the
  category codes and the command codes that share their numeric values; §135
  fixes the glue orders; §305 fixes `scanner_status`; §289/§365 fix how a
  token is represented. Umber's Rust variants are `Superscript`, `EndLine`,
  `Parameter`, `Ignored`, `Active`, `Invalid`; tex.web's names are `sup_mark`,
  `car_ret`, `mac_param`, `ignore`, `active_char`, `invalid_char`.
- **The catcode table and the character-command table are different tables.**
  They agree only at codes 1..8 and 10..12. Catcode 0 is `escape` but command
  0 is `relax`; catcode 9 is `ignore` but command 9 is `endv`; catcode 13 is
  `active_char` but command 13 is `par_end`; catcode 14 is `comment` but
  command 14 is `stop`; catcode 15 is `invalid_char` but command 15 is
  `delim_num`. Keeping one table and reusing it for both is a naming defect
  that will look correct on most inputs.
- **A Rust `Debug` rendering must never reach an observation payload,** and
  never round-trip through one either: `Debug` spells Umber's variant names
  and the oracle spells tex.web's, so any agreement is accidental. A
  `format!("{x:?}")` in a record field, or a transport that prefix-matches a
  record field against a Rust variant name, is a bug on its own terms
  regardless of whether a fixture currently notices.
- **Each family gets one total function with a single entry point.** No
  silent catch-all: an unnameable value either has an explicit,
  tex.web-cited arm or gets a deliberate name no engine installs
  (`undecodable_meaning`, `uncommandable_character`) so that it looks like the
  internal defect it is. Adding a `Catcode`, `Order`, or `ScannerStatus`
  variant must fail to compile until it is placed deliberately.
- **Never re-derive a name a producer already computed.** A transport that
  recomputes a command name from the spelling instead of carrying the
  producer's is a second, divergent table; it will silently mask engine
  divergences that the producer's name would have exposed, which is exactly
  what a spelling-derived command name did until umber2-johp.141 deleted it.
