# tex-command Guidance

Read the repository-level `AGENTS.md` and `docs/tex_command_core.md` before
editing this crate.

## Crate Role

Let and futurelet scan through separate entries for their real TeX82 grammars;
there is no boolean-selected binding frame or futurelet forwarding helper.
Math-font family selectors consume the actual restricted integer directly;
the unused family request and math-request variant are absent.
Math style heads have no scanner operand or request variant; execution appends
its actual node style directly, and the duplicate `MathStyleKind` is absent.
Defined math symbols also append their final noad directly; the scanner no
longer constructs a request for their already-complete code. Numeric mathchar
and delimiter heads scan their real restricted integer in native main control;
the duplicate Character/Delimiter math requests are absent. Remaining math
operand dispatch selects its actual scanner in the executor's one mode/command
switch. MathRequest, duplicate noad/limit/equation selectors and the mu-material
wrapper are absent; script markers carry their actual kind.

Executor group-open consumers call the canonical `scan_left_brace` directly.
The discarded discretionary opening wrapper and its scanner adapter are deleted.
PDF font actions scan their actual font and balanced text in main control.
Their immediately consumed text renders directly from attempt storage into
final bytes; the discarded `ScannedPdfFontAction` and scanner adapter are
removed. PDF object references scan their actual integer directly; their
discarded result wrapper is absent. Showbox uses the actual profile-register
scanner; its discarded index/provenance adapter is removed. Box/copy operands
likewise read the actual register index without a `ScannedBoxRegister`. Read
scanning returns its actual target and final definition owner. The unused
open/close/read request union and retained stream/scope columns are deleted.
Text accents execute their real assignment/base grammar in main control;
`ScannedAccent` and `ScannedAccentBase` plus their scanner adapters are absent.
Left/right/middle scan actual delimiters directly; the duplicate boundary
request, kind and scanner wrapper are deleted. There is no intermediate retained token list for font-map or
ToUnicode text.
Recovery entry points consume native spelling plus a genuine exceptional record
only when present; math-shift, paragraph and off-save backup share that native
input seam. No recovery command record is built for ordinary spelling.

The bulk character reader resolves its boundary once and offers accepted words
directly to the main-control consumer. Only declined or exceptional boundaries
construct a command record. Native acceptance preserves fuel charging, source
positions and literal-brace accounting before semantic consumption.

`tex-command` is the sole target owner of canonical source tokenization, input
levels, raw command delivery, expansion, macro calls, conditions, scanners,
alignment delivery, and static profile dispatch. It depends on `tex-state` and
must never depend on `tex-exec`; `tex-exec` consumes its completed
unexpandable commands.

`docs/expansion_memory_lifetimes.md` is the current implementation map and
retention audit for command-state generations, execution scratch, macro and
scanner nesting, input ownership, and checkpoint replay. Update it whenever this
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

Dimension and glue scans return their actual scaled value or glue specification,
without scalar wrappers or unused first-token origins. Infinite dimensions also
return their actual order. Integer and internal-value diagnostic origins remain
with their real consumers.

Executor-facing scalar scans return their actual typed product or error. Generic
scalar result unions, publication frames and projection/status APIs are absent.
Nested scalar calls return their actual value or error directly. Large command
error payloads are boxed when raised; no scalar call frame, status handoff or
optional error slot is constructed for successful scans.

Expansion continues inside the shared reader until its actual consumer returns
a command or completion; there is no expansion-candidate carrier. Macro
activation and primitive expansion continue that reader’s semantic back edge,
which charges every newly consumed token once. Cold input transitions retry
under the existing charge.

Keyword, optional-equals and numeric-digit scanners keep their synchronous terminal consumer
inside the same reader across accepted characters and leading spaces. They
stop at the actual operand completion or a cold EOF/replay boundary, resetting
expanded-delivery state for each accepted terminal. Single-command delivery is
another static consumer of this same loop. Continuing scanners lend their
synchronous callback to one shared scanner-call boundary, which enters the same
reader without copying it for each scanner closure. Single-command delivery
returns directly; no heap callback or second reader exists.

Resident input completes directly into its consumer or leaves the authoritative
row/cursor at a real source, exhaustion or parameter boundary. There is no
input-transition descriptor or immediately consuming transition dispatcher.
Actual cold input owners read that row after the span borrow ends. Parameter
substitution advances once before pushing; consumed character prefixes return
before exhaustion/retirement, and MacroArgument words remain raw.

## File Map

- `Cargo.toml`: dependency-light crate manifest and boundary-test support.
- `src/lib.rs`: intentionally small public facade and private module tree.
- `src/attempt.rs`, `src/attempt/token_lane.rs`, and `src/attempt/tests.rs`:
  command-lifetime token scratch in one fixed-chunk lane and one row table. An open collector and its sealed list use the same row; sealing changes only its phase. Coordinates reject foreign arenas, consumed or stale reused rows, and writes through a sealed buffer alias. Row serials advance locally without rollback; only arena creation allocates a global identity. There are no attempt definition/glue/provenance/name lanes, parallel origin arrays, contiguous word copies, or mixed promotion protocol. Truncation returns whole chunks. Macro invocation storage does not use this
  arena. Sealed list coordinates are move-only: publication, insertion, rendering
  and splicing consume their roots and return their chunks immediately. Failed
  scanners truncate their serial-frontier suffix; there is no command cleanup.
  A resource miss halts the command and
  the host replays from a full checkpoint.
- `src/execution_scratch.rs`, `src/execution_scratch/tests.rs`: reusable
  bounded expression frames and admission/sinks for ordinary resident runs.
  Definition and csname collectors consume those runs directly. One stack-local admission cursor transfers into the resident reader; suppression restricts it in place without copying its configuration. Admission
  borrows the final sink’s scanner cursor, settles each accepted word and writes it in one traversal;
  no accepted-prefix rescan or separate span-copy pass remains. Execution scratch owns
  no duplicate scan depth, macro argument payload, or publication capability.
- `src/input/arguments.rs`, `src/input/arguments/tests.rs`: final macro
  `ArgumentArena`, with one reusable vector per simultaneous invocation. Up to four immutable definition words write directly to the final pool without acquiring a source owner; larger spans retain their actual source. Scalar capture queries the source row only for an actual retained span and advances local visible length directly.
  Each region holds at most nine final ranges; call-local `MatchedArguments` transfers only its owner coordinate, and `ArgumentCursor`
  holds brace/group facts and the unpublished delimiter suffix. Publication
  transfers the region to the macro body without copying words or ranges; no
  separate parameter vector or body parameter-base/count cache remains. One
  live parameter count preserves native capacity/high-water bookkeeping; parameter
  rows borrow it. Parent retirement and recovery input have independent
  lifetimes. Every Result exit releases an unfinished capture, and body
  retirement returns the region. Named captures exclude the allocator pool;
  restore clears abandoned owners and retains reusable engine capacity.
- `src/scanners/scalar/numeric.rs`: native integer, dimension and glue entry consumes leading signs and substantive meaning directly in the shared reader; internal probes and canonical backups have one implementation, and number conversions share this scanner. No leading-command handoff or conversion-specific digit reader remains.
- `src/scanner_kernel.rs`: stack-local synchronous scanner cursor shared by
  macro arguments and `scan_toks`. It keeps brace and first-token facts beside
  the caller-owned output sink and settles each accepted word inside plain
  run admission and final writing; it owns brace depth and a three-state outer-group candidate, with no per-word count or unused paragraph flag. A resource miss unwinds this cursor and the host replays from a full
  checkpoint rather than promoting scanner state into retained scratch.
- Macro matching saves only its actual prior scanner status, with no installed-status episode packet or activation-class product. Empty delimiters produce no literal view; observed transitions and overlap projections are demanded at their consumer.
- Ordinary expansion has no parked-expansion owner. The shared reader exposes
  token facts; unobserved macros activate from resolved invocation facts without
  constructing a command. `CurrentCommand` is the one request-local
  delivery/scanner/observation/recovery values. Resource misses unwind consumers
  before host-owned full-checkpoint replay.
- `src/scanners/structured/token_requests.rs`: token-field visitation and
  consuming mapping on genuine generic scanner requests. PDF/image/balanced
  text requests use attempt token ids while scanning and native token ids
  after publication; input-read requests similarly use their actual definition
  field. There are no duplicate rooted request shapes or phase placeholders.
- `src/host.rs`: borrow-scoped, nonserializable host-capability boundary.
  Long-lived immutable resources remain in `CommandHostCapabilities`; live
  executor mode, auxiliary, and effective-tail facts cross the synchronous
  `CommandHostFacts` provider only when their exact scanner or conditional
  consumes them. The processor retains the provider's concrete type, and initial
  facts are zero-sized. A completed consumer can lend the real mutable execution
  owner alongside its context; only genuine cold replay reborrows erase the
  provider type. No runtime initial/borrowed enum or fact cache enters command
  state or a replay round. Native semantic callbacks use `with_execution_owners`
  to lend the actual capabilities, observer, provider and matching decline owner;
  no copied host or substitute ledger is installed.
- `src/profile.rs` and `src/profile/tests.rs`: public semantic character values,
  immutable command/character profiles, the distinct canonical compiled-engine
  semantics that survive loading an older format, capabilities, stable
  fingerprints, and focused value/identity tests.
- `src/state.rs`: exclusively mutable live aggregate command roots,
  persistent command state, retirement-owned executor replay-completion
  fences transferred directly to §325/§390 descendants, current-generation
  execution scratch, pending afterassignment, and direct semantic mutation. It owns no group topology or aftergroup lane. `CommandProcessor::end_group` consumes the native state stack and links aftergroup words directly into final backup input; restoration callbacks run synchronously.
  Main control holds this actual state directly; the retired interpreter wrapper
  remains absent. Profiling-only construction and processor-loan counters attach
  to the real owners, with no ordinary runtime bookkeeping.
  `src/state/attempt_transition.rs` owns synchronous child scopes
  and halted full-checkpoint abandonment;
  `src/state/projection.rs` owns allocation-free retained-byte
  and dependency views; `src/state/executor_publication.rs` owns ordered
  executor-facing fact transfer and demand-only queued-list publication. Each is an inherent implementation on the
  same authoritative `CommandState`; none adds a facade or state owner. A named
  checkpoint copies the quiescent transient roots by value; warmed command
  delivery performs no root admission and writes no journal. A resource miss
  halts the current operation and the host restores a full checkpoint before
  ordinary delivery starts again; no command-owned retry package or cursor
  crosses that boundary. The live `CommandState` also owns TeX82's three
  scalar stack maxima directly; they are operational session evidence outside
  snapshot roots and survive restore without shared synchronization.
- `src/command.rs`: actual delivered word, origin and site fields, native
  `MeaningWord<G>` from the authoritative eqtb (static meaning, macro key/flags,
  or full font capability), and public opaque, ephemeral `CurrentCommand` containing that same packed
  word. Its native control-code projection drives the sole executor switch;
  decoded meaning is materialized only at a consuming boundary.
  Command-sensitive boundaries materialize this pair from the reader's facts
  and already-resolved meaning. Ordinary macro activation and unexpanded body
  collection construct no command record. The executor-owned main-control
  consumer also executes admitted relax words within the canonical reader,
  without a command record or post-fetch continuation. The same native value crosses raw,
  expanded, scanner and execution boundaries; observation and diagnosis project
  their genuine consuming shape without a second command representation.
  Exact source geometry remains behind the spelling's
  packed origin and is materialized only by cold processor consumers; the hot
  value retains only source-role policy and direct-line facts. The executor then borrows the one caller-owned value through delivery and scanning,
  and moves it only into another semantic owner; it never
  enters a durable snapshot or format boundary.
  Command identity is derived at diagnostic/observation consumers from the native word and existing frozen-noexpand flag; no duplicate identity column is materialized during ordinary delivery.
- `src/command/native_meaning.rs`: demanded class, primitive and scalar
  projections on the actual delivered command. Diagnostic preflight can borrow
  its actual native meaning directly without a decode/re-encode handoff. No separately classified
  command-word type or cached class is constructed by resolution.
- `src/command/identity.rs`: demanded diagnostic/observer command identity and
  canonical conversion/xray selectors on that same actual command.
- `src/direct_command_delivery.rs`: profiling-only mixed-meaning structural
  harness for direct dense-row writes into the caller-owned command. Its
  one/4,096-round gate counts row accesses, row decodes, final macro-owner
  acquisitions, whole-meaning/command copies, and warmed allocations without
  adding production state or an alternate delivery path.
- `src/processor/expand_structural.rs` and `src/conditionals.rs`: name
  characters and numeric relations consume native meanings directly in the
  shared reader. The name collector retains bulk resident-character admission
  and rejects control-sequence aliases under TeX82 §372. A single outlined
  relation scanner serves ordinary and recursive conditions; no compact
  command-scanning duplicate exists. Missing delimiters capture their actual
  diagnostic site and back up before the report receives that site.
- `src/processor/expand/collect.rs`: raw collection and `get_token_spelling`
  consume tokens in the canonical reader. Raw operand consumers receive actual
  spelling and demand-only native meaning in one continuing reader; ordinary
  unobserved operands construct no command. Let returns the native meaning word directly for installation;
  futurelet releases its first reader before the second operand and backs
  both words through the common canonical insertion implementation. Observation,
  outer recovery, alignment and source control-sequence creation keep their
  original semantic positions.
- `src/processor/expand/tests/kernel.rs`: nested reader resumption, every
  bounded fuel cut under observed/unobserved delivery, observer attachment
  between calls, live meaning resolution after backup, and clearing a reused
  raw destination at EOF and fuel failure.
- `src/processor/expansion_depth.rs`: Web2C's shared primitive/expression
  depth capacity and unconditional restoration on Rust result unwinding.
  `src/processor/expand/tests/nesting.rs` covers the five nested operand
  families at exact capacity boundaries and an explicit native-stack scaling tier.
- `src/processor/expand/tests/resident_runs.rs`: consumer-stop resumption,
  exact fuel-failure prefixes, and one reader selection per resident run.
- `src/processor/expand/resident.rs`: runs scalar character admission inside one
  storage selection, calling the selected scalar consumer at a boundary. Raw delivery
  uses the same reader with single-word or continuing admission. Directly borrows the exposed semantic
  input frame and advances its existing physical reader. Input frames own
  reader state across nesting and rollback; delivery has no parallel cached
  storage tag or reader invalidation protocol. Source-token delivery enters
  the normal source reader; only refill, exhaustion, and recovery use cold
  input transitions.
- Unicode source admission validates immutable backing once. The line cursor
  reads ASCII directly and decodes only the next non-ASCII scalar, retaining
  exact byte/scalar coordinates without revalidating the remaining file.
- Macro argument trace markers are read by the demanded cold trace, and
  observation lengths are derived only inside the observer boundary. Ordinary
  matching publishes its final cursor without these discarded trace inputs.
- Main-loop character consumers borrow the actual host owner through the same
  processor as native command delivery. The source and resident admission seams
  lend state, fuel, effects, and that owner synchronously; no temporary
  forbidden-facts processor or second character-mode loan remains.
- `src/processor/expand.rs`: consumer-directed delivery and static primitive
  dispatch. Raw callers, expansion, collectors, and main-control text share one
  input/fuel authority; observation specializes at synchronous entry. Final
  delivery and exceptional settlement retain one native CurrentCommand slot;
  printed identity and spelling are demanded by their actual consumers.
  Macro chains remain iterative; failure unwinds for host replay.
- `src/processor/expand/input.rs`: source reads, borrowed source-character
  admission, and cold line/retirement/recovery transitions. Reads invoke the scalar consumer
  without constructing commands or publishing backup authority. Source
  reads stage a file diagnostic coordinate; settled raw delivery publishes it
  only when §342 reaches a command exit.
- `src/processor/expand/consume.rs`: one dense meaning resolution, ordinary
  macro activation from invocation facts, and synchronous native terminal consumption.
  Unobserved expandable primitives also invoke their shared engine directly from
  the opcode and original spelling, without a temporary command. Observation,
  suppression, outer validation, alignment and template sentinels keep their
  actual settlement boundary. One read/interpret loop invokes native consumers directly or initializes explicitly requested delivery records. Matching's local
  storage lives in its activation call; prefix diagnostics consume spelling and
  invocation facts without a command or delivery geometry. Resolution returns
  only demanded control-sequence identity; literal and lookup facts stay in
  the original input word. Raw command construction returns no metadata.
- `src/processor/expand/skipping.rs`: TeX82 §494's continuing raw consumer.
  Source and resident words update actual brace/conditional nesting directly;
  only an unmatched delimiter completes the skip. Ordinary skipped words have
  no command or effect product. Observation, suppression, outer recovery and
  alignment construct their demanded command inside this same consumer.
  The shared reader reserves fuel across cold transitions and charges each
  additional resident word before advancing the authoritative frame.
- `src/processor/expand/primitive.rs`: the sole primitive expansion engine from
  native opcode, traced invocation spelling and control-sequence identity. It
  retains capacity and resource unwind, outlines real conditional/nonconditional
  recursion, and creates print values only for demanded tracing. Conditional
  delimiter diagnostics and input-name recovery consume native spelling at
  their actual exceptional boundary. Delivered/observed callers use this same
  engine from their existing command's invocation facts.
- `src/processor/expand/scalar.rs`: synchronous continuing scalar and main-control consumers borrow native spelling and meaning directly; exceptional observation/recovery lends its demanded record. Accepted words construct no command. The main-loop lookahead role preserves §1038's raw character delivery and expands only noncharacters in the same reader. Mixed macro/noexpand tests compare every bounded fuel cut and observed delivery order against destination-directed lookahead.
- `src/processor/expand/once.rs`: TeX82 §368's single-expansion consumer of
  the same reader. It creates source names while reading its operand, restores
  normal delivery before nesting, activates ordinary macros without a command,
  and returns after one expansion or one unexpandable-token backup. Matcher
  failure unwinds before the deferred first operand is replayed.
- `src/processor/expand_structural.rs` and `src/conditionals.rs`: name
  characters and numeric relations consume native meanings directly in the
  shared reader. The name collector retains bulk resident-character admission
  and rejects control-sequence aliases under TeX82 §372. A single outlined
  relation scanner serves ordinary and recursive conditions; no compact
  command-scanning duplicate exists. Missing delimiters capture their actual
  diagnostic site and back up before the report receives that site.
- `src/processor/expand/collect.rs`: raw-spelling consumers for macro matching,
  deferred operands and unexpanded replacement words. They share input and
  exceptional settlement, append through the existing collector grammar, and
  construct a command only for observation, semantic exceptions, recovery or
  actual backup. Raw-token consumers receive final spelling and origin, with a
  command borrowed only at those demanded cold boundaries.
- `src/processor/expand_structural.rs`, `src/processor/expand_input.rs`, and
  `src/processor/expand_convert.rs`: direct/static structural, source, and
  TeX/e-TeX conversion primitive families. They borrow the one processor and
  mutate its authoritative state in place; none owns a command facade or
  alternate delivery path.
- `src/processor/expand_pdf.rs`, `src/processor/expand_pdf_string.rs`, and
  `src/processor/expand_pdf_file.rs`: direct/static pdfTeX state/object,
  string/regular-expression, and immutable-file enquiry primitive families,
  using the same synchronous scanner calls and host-owned replay boundary.
- `src/processor/expand_replay.rs` and `src/processor/expand_render.rs`:
  expansion-result insertion plus shared append-oriented TeX command and
  conversion rendering. These contain no expansion driver or semantic owner.
- `src/processor/mod.rs`: processor construction plus the episode-local
  observation delivery cursor used at a direct executor boundary; it restores
  observation ordering but owns no command/input semantics or retry state.
  There is no delivery stamp or backup freshness proof: §325 `back_input`
  accepts the delivered command it is handed, exactly as TeX's `cur_tok`, and
  ordinary delivery publishes no freshness census. Focused fixtures derive their
  known delivery volume at the measurement boundary. `CurrentCommand` owns no observation
  sequence; a resource miss drops the cursor with the call-local processor and
  full-checkpoint replay starts a new observation episode.
  Every construction uses `CommandProcessor::new`, which takes the
  caller-owned admitted context, session-owned fuel, observer, and
  operation-local diagnostic-effects collector directly and constructs no
  temporary owned ledger or whole-context handoff.
  Its scoped native callback reborrows the real context, command state, effects, and fuel after synchronous scans return; sealed scanner products are consumed at their actual destination without a completed-command stage. Actual Drop still owns resource-failure cleanup.
- `src/command.rs`: test builds count resolved-command writes for structural
  assertions. Profiling builds derive that exact one-write-per-raw-delivery
  volume from the singular command-work ledger, so the resident loop does not
  maintain a parallel ownership census. Genuine backup copies remain separately
  counted because they are not implied by the raw-delivery vector.
- `src/error.rs`: compact native command failure identities. Fuel work,
  filenames, resource failure and source annotation each have one cold payload
  owner allocated only when raised; resource packets move through executor and
  session without repacking. It also owns the shared dimension-scanner recovery
  diagnostic vocabulary for the sole canonical scanners.
- `src/fuel.rs` and `src/fuel/tests.rs`: checked finite command-work limits,
  one exact remaining-budget countdown, profiling-only non-fuel work
  counters, terminal/publication-time consumed-fuel and raw-frame derivation
  (the latter is the sum of the exhaustive raw-owner vector), the
  constructor-free borrow-only `CommandFuel` capability used by leaf
  operations, and the top-level `CommandFuelLedger` session owner shared by
  every canonical processor episode. Default production delivery stores and
  updates only the fuel guard; the existing `profiling` resolution compiles in
  the exact detailed census. Fuel and profiling counters are operational
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
  typed file diagnostic columns distinct from immutable token locations, and
  focused line-contract tests. TeX82 §363 lends a valid normalized UTF-8
  slice from the resident backing directly through `firm_up_the_line`; only an
  invalid exact-byte display projection or a genuine terminal replacement
  materializes ownership, and the latter remains the source slot's sole
  `line_backing` owner.
- `src/input/tokenizer.rs`, `src/input/tokenizer/tests.rs`: canonical
  token-at-a-time exact-byte and separately identified UnicodeExtended M/N/S
  tokenization, semantic control-sequence spelling, direct production
  emission of ordinary characters directly into packed identity and provenance,
  with concrete native queries and no general SourceToken projection. Control
  words complete into their borrowed-text or transformed-name consumer without
  a ScannedControlSequence union. Structured SourceToken values are produced
  only for lexical inspection. Physical spelling checks are reused within
  each control-name character;
  profile-specific superscript notation,
  invalid-character recovery steps, byte/scalar ranges, and focused conformance
  tests. A cursor with no loaded line returns `NeedLine`; physical acquisition
  belongs to the singular input-top owner and never receives a backing registry
  through the ordinary token path.
- `src/input/levels.rs`, `src/input/levels/tests.rs`: input rows and admission
  from actual traced words, stored-list owners and scratch-list owners, plus
  genuine replay iterators. No general source enum or duplicate admitted extent
  is constructed. The common packed frame owns identity, source context, start, position,
  limit, kind, and delivery/retirement flags. `StoredList` owns its immutable
  `TokenListId`; `MacroBody` owns its `ResidentMacroBody`, invocation facts, and
  transient argument coordinates. Both read retained spans without activation
  copies. Insertions, backups, and attempt lists use the
  input stack's traced-word suffix; matched arguments use body-owned pooled
  final regions. A source row owns one boxed `SourceSlot`
  with backing, line cursor, `everyeof`, ancestry, and retirement state.
  `open_depths` is `\tracingnesting`'s record, borrowed before retirement.
- `src/input/history.rs` and `src/input/history/tests.rs`: the authoritative generation-tied `InputStack`.
  The live stack is the only mutable copy; a checkpoint copies it by value at
  quiescence, so rows carry no rollback marker and no transition records an
  inverse. Source-owner changes release displaced owners directly.
  Capture retains immutable row owners and copies transient words and frame
  coordinates as one aggregate. Named admission rejects live macro activations.
  Retirement drops the row owner, returns its argument region, and truncates
  its parameter or ordinary input suffix exactly once; restore releases
  displaced rows and clears their argument regions. Diagnostics borrow the retained
  parameter/replacement span even after its defining group retires.
  `processor/expand/resident.rs` selects a row and its storage once per run,
  advances the owning frame, and calls a statically selected consumer with word
  and origin scalars. Parameter substitution, exhaustion, and source acquisition
  return only genuine cold transition facts. No token/result carrier or cached
  reader enters command state.

- `src/input/stack.rs`, `src/input/stack/tests.rs`: exact input retirement.
  Ordinary exhausted token, macro-body, and macro-argument rows retire the
  already-selected resident coordinate directly, release their one storage
  owner, and settle observation/alignment/replay completion from copy-small
  row facts without a retired-level projection or a second pop path. The same
  transition retires a v-template after successful `do_endv`; terminal token
  input, pre-`do_endv` retained
  v-templates, source EOF, recovery, and parameter results retain
  no command-slot borrow; an ordinary final result has already resolved
  meaning, applied one-delivery suppression, classified only required
  alignment work, and returned no command reference. The files also own the singular
  physical-line acquisition owner, the canonical frame-push transition and scalar maximum
  update, centralized token-stack admission, retained v-template lifecycle,
  scalar active macro-body/parameter accounting, macro-activation cleanup,
  borrowed source-ancestry comparison with copy-only
  retirement facts, `param_start` parameter replay ownership, and
  trace-independence tests.
- `src/input/mod.rs` and `src/input/tests.rs`: tex.web §§310--318's live error-
  context traversal and omission matrix. The traversal selects the current,
  `\errorcontextlines`-budgeted, and bottom levels before pseudoprinting; it
  retains only one deferred bottom coordinate and never materializes omitted
  level strings.
- `src/processor/`: public borrow-only processor facade with one concrete
  raw/expanded destination loop,
  expansion, scanner-status, and alignment orchestration. The fused
  destination pipeline advances resident input and inspects the command
  in place. The processor loop itself owns the resident frame and cursor, so
  parameter substitution and ordinary exhaustion
  reselect the authoritative top only after changing the input stack, while
  source line acquisition remains the cold resource boundary. Its sole
  delivery settlement applies noexpand, outer
  validity, alignment classification, and observation after dense resolution
  has ended. The loop owns canonical token-to-current-meaning delivery;
  creation permission exists only at source tokenization and is absent from
  delivery controls. The loop tests raw versus expanded only after one command
  has settled; resident input advancement itself has no mode branch.
  token. Only cold resident transitions leave the frame loop for a handler.
  Expanded delivery classifies each resolved meaning once as return,
  end-template handling, macro activation, undefined recovery, or one exact
  primitive dispatch. That borrowed decision directly drives expansion;
  expansion does not match the resident meaning or primitive opcode again.
  Scalar consumers return their own native completion directly. Only true
  EOF/replay transitions construct their terminal result; ordinary words have
  no second completion wrapper. Exceptional word settlement owns diagnostic
  rollback snapshots and error payloads; ordinary settlement updates only
  actual source position and literal brace state.
  Main control enters the same canonical expanded word loop and one actual
  expansion episode. `get_x_token` (§380) converts frozen `end_template`
  to `endv` in place; raw character lookahead uses §381 `x_token`, whose
  §366/§376 expansion backs and rereads the actual frozen `endv`. Alignment
  delimiters retain their separate v-template transition. Raw character lookahead is its
  genuine TeX fetch label, not a first-read preflight or eligibility handoff.
  `status.rs` owns the one processor-level scanner episode mechanism for
  typed status entry, observation visibility, recovery re-entry, and complete
  prior-state restoration. Macro matching instead saves only the actual prior status through the same native status owner and derives its demanded observed exit from the existing macro identity.
- `src/processor/next.rs`: public raw-delivery entry points plus
  demand-only resident-command observation after all dense borrows end. The
  fused state machine in `expand.rs` delegates cold source retirement, outer
  recovery, and alignment interception directly to their private semantic
  modules without introducing another command or input owner.
- `src/processor/end_input.rs`: physical-line acquisition, source exhaustion,
  `\everyeof`, final cleanup, terminal/v-template retirement, and explicit
  §§325/390 stack conservation. It is the cold processor retirement boundary;
  ordinary delivery retirement stays inside the resident input transition.
  Stack conservation tests `loc=null` from each admitted row's scalar position
  and limit; it never reclassifies a storage domain, indexes replay, or reads a
  definition word merely to decide exhaustion. The v-template exception uses
  the actual packed kind directly, without constructing token behavior.
- `src/processor/backup.rs`, `src/processor/recovery.rs`, and
  `src/processor/outer_recovery.rs`: exact command backup/replay insertion,
  executor-facing recovery operations, and scanner-status outer/runaway
  interception respectively. They share the live processor directly and add
  no recovery queue, command slot, or runtime dispatch layer.
- `src/processor/alignment_interception.rs`: alignment-aware delivery entry,
  typed delimiter/v-template handoff, and active-cell input proofs over the
  canonical alignment state; it never reclassifies input in the executor.
- `src/processor/tests.rs`: tracked command-root publication and fail-closed
  unsupported-boundary coverage.
  Profiling consumers use actual input/scratch owners; the retired matched-word
  read projection and namespace-promotion allocation/layout tests are absent.
- `src/processor/alignment.rs`, `src/processor/alignment/tests.rs`: canonical
  alignment-delivery state and focused stack, brace-depth, template, and omit
  lifecycle tests.
- `src/processor/expand/tests.rs` and `src/processor/expand/tests/`: shared
  expansion-test setup and named delivery, balanced-body, numeric/conditional,
  expression, font/name, macro, main-control, character-run, protected-replay, and
  PDF-query families. Existing kernel, nesting, and resident-run modules stay
  in the same unit-test target; `src/processor/fixtures/` holds bounded source
  microfixtures.
- `src/scanners/`: private typed scanner family. `hyphenation.rs` owns TeX82
  §934/§960's `\hyphenation`/`\patterns` scans, which are `get_x_token`
  classification loops rather than `scan_toks` collections and so must never
  enter the `absorbing` scanner status or track a brace depth.
  `expression.rs` and `expression/tests.rs` own e-TeX 2.6's four typed
  expression scanners and two glue-level conversions, including their
  explicit parenthesis stack, arithmetic recovery, glue-order rules,
  observations, and checkpoint-retry tests.
  `structured.rs` retains the public reexports from private
  `structured/*_values.rs` result modules. `structured/internal_state.rs`
  holds shared local-progress declarations. `structured/pdf_*.rs`, `math.rs`,
  `boxes.rs`, `alignment.rs`, `streams_fonts.rs`, `character.rs`, `write.rs`,
  `definitions.rs`, and `filename.rs` each implement one command-processor
  scan family. They share the existing processor scanner episode, delivery,
  error, and checkpoint-replay authority; see
  `docs/structured_scanner_ownership.md`.
  `structured/character.rs`'s `scan_accent_base` is deliberately a one-command step
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
  sign/provenance, radix-tail accumulator, or completed §442 character code in
  call-local state while the ordinary scanner performs its optional-space
  probe. A resource miss unwinds that local state and full-checkpoint replay
  re-enters the canonical TeX82 §§440--445 token probe. The terminator test is
  on the command, so it is the category code and never
  the character: §207 makes `spacer` the command a category-10 character
  carries, and §349 is what normalizes such a character's `cur_chr` to a
  space inside §341's `get_next`. The same canonical scanner grammar handles
  optional-equals, borrowed keywords with bounded matched prefixes, integers, dimensions, glue,
  filenames, internal values, expressions, and font selectors. Scalar state is
  call-local; it is never copied into an executor phase or retained across a
  resource boundary. Ordinary dimensions, glue coefficients and expression
  shortcuts share one synchronous decimal/unit grammar; no dimension progress
  enum, stage cursor or duplicated staged unit scanner remains.
  Character tests project native character/category facts directly. The last
  integer terminator retains only those facts for decimal and infinite-unit
  lookahead, rather than a duplicate whole decoded meaning.
  Success and error cleanup close local scopes deepest-first,
  while a resource miss unwinds the whole call tree and the host replays from a
  full checkpoint. Do not add a root mailbox, caller-order result tape,
  destination inference/search, or command redispatch fallback.
  `filename/tests.rs` owns focused expanded filename scanning, termination,
  replay, and registered-source retry tests.
  `structured/math.rs`'s `scan_math_field_episode` is TeX82 §1151's `scan_math`,
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
  only store-local installation mechanics. `registry/tests.rs` checks the
  frozen null-font identity and missing-only format registration repair.
  `catalogue.rs` defines the
  behavior-free descriptor vocabulary and exhaustive validation. Execution
  dispatch remains handwritten.
  `prefixed.rs` owns TeX82 §209's `max_non_prefixed_command` partition -- the
  single test §1211's `prefixed_command`, §1270's `do_assignments`, and the
  `\global` prefix all make. It is narrower than "this command assigns
  something": `\begingroup`, `\endgroup`, `\aftergroup`, `\afterassignment`,
  `\openin`/`\closein`, and every `extension` primitive are excluded, so a
  caller that starts from a broader notion and subtracts them by hand is
  re-deriving this predicate one exception at a time.
- `src/macro_call.rs`, `src/macro_call/arguments.rs`,
  `src/macro_call/diagnostics.rs`, and `src/macro_call/tests.rs`: native macro
  activation, the canonical argument grammar, and actual tracing/recovery
  diagnostics. The matcher borrows definition-owned parameter patterns and
  delimiter failure links. Empty delimiters have no borrowed literal view, and calls construct no activation-class product, `MacroPlan` or failure table.
  Ordinary words enter the grammar directly without a command projection;
  actual cold settlement and error backup own the demanded command. One scratch
  writer retains argument holdback and source origins until activation transfers
  them to the input stack; focused tests cover scalar recovery, overlap, group
  trimming, nested replay, and source-origin forwarding. Ordinary parameterless activation never enters the scanner/matcher call frame; only the actual pattern or exceptional invocation owns the call frame. Matching restores its prior status and releases untransferred argument owners at their actual return.
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
- `src/token_collector.rs`: scanner-owned in-progress collector for token-list
  and definition scans. It owns their one raw packed-spelling classification,
  destination phase, and publication cursor. Macro argument matching reuses
  only traced spelling; its purpose-built writer and invocation-owned words
  and ranges live in `input/arguments.rs`, with no generic destination dispatch.
- `src/macro_call.rs`, `src/input/levels.rs`, and `src/input/arguments.rs`:
  activation retains the final definition body and transfers the invocation's
  final argument region, including its at most nine per-parameter values.
  The parameter loop creates each actual argument cursor and lends it to either
  scanner, which returns only completion or failure. No cursor result is copied
  out and immediately consumed. Argument sinks count the words accepted by the canonical admission loop
  and extend their retained source or pooled spelling directly.
  A `MacroBody` row borrows the sealed replacement; body retirement subtracts
  the region's native parameter count and returns the whole owner. There is no
  copied parameter vector or frame parameter-base/count cache.
  An out-parameter word pushes a `MacroArgument` row over its range.
  Parameterless macros create no matcher or argument state, and ordinary
  empty replacements retain no exhausted input row.
- `src/scan_toks.rs`, `src/scan_toks/expanded.rs`, `src/scan_toks/tests.rs`: private canonical token-list
  scanner and focused parameter, collection, expansion, scanner-status, and
  recovery tests. A scanner owns no arena or scope. Temporary collection uses
  the scanner word/builder lanes. Ordinary `\def`/`\gdef` write semantic words into the arena's one build
  lane; failure clears it and sealing publishes one exactly sized span with
  its final provenance. `ScannedMacroDefinition` moves the actual publication
  owner to assignment; allocation has no local/global destination. The collector's
  phase tag is the one authority for parameter versus replacement writes; no
  destination-local phase mirror or published-header provenance edit exists.
  Raw definition scanning is entirely call-local. Expanded replacement collection
  is a synchronous consumer of the same expansion reader as ordinary delivery;
  its borrowed sink and scanner episode handle direct splices, protected retention,
  replacement grammar and outer recovery. Unobserved macros activate directly
  from invocation facts, without a temporary command or a separate get-x loop.
  `\edef`/`\xdef` retain only the build key while their ordinary local scanner
  runs; a resource miss unwinds the builder and expansion call tree. `read_toks`
  uses that same direct native writer and moves the sealed publication owner straight to assignment. Runaway reporting borrows the incomplete replacement. Read setup and finalization share one cleanup transaction which returns and aborts the actual writer, restores `align_state` and scanner status, and truncates its exact token scratch suffix on every error. General token-list scans append directly to independent
  branches in the attempt's shared fixed-chunk lane; they never own or recycle
  a per-scan word vector, and sealing is a metadata-only publication. Nested macros
  use separate macro frame/argument lanes, so push/pop never interleaves their
  scratch with scanner output. A synchronous scan uses branded frame indices
  only while its local call is active. Scratch-frame insertion preflights
  allocation, capacity, and serial advance before admitting that local scan;
  failed insertion or a resource miss aborts the nested child deepest-first,
  finishes scanner status, and reclaims both the scanner suffix and its
  parent-owned sink rows while retaining reusable lane and builder capacity.
  Its semantic
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
  by-value command checkpoints. `snapshot.rs` owns `CommandCapture`, the
  serial-ordered `CommandCheckpoints` list, its single candidate fork, and the
  public `CommandSummary` naming one `(owner, serial)` capture. Each capture
  copies the quiescent input levels, replay rows, conditions, pending afterassignment token and alignment state. Native groups and aftergroup saves belong to tex-state and are forbidden at publication;
  its retained bytes are charged under its own aggregate owner key.
  `boundary.rs` owns quiescence validation, publication, restore (which drops
  every newer capture), release, editor-source rehome, and the
  fork/reject/accept transaction. A fork moves the live roots aside as the
  accepted head and detaches the newer captures; reject moves both back,
  accept drops them. Restore abandons the halted operation and clears the
  attempt arena, because every capture is taken while it is empty. Validation
  never mutates the runtime, aggregate command roots are not `Clone`, and
  capture requires quiescent execution scratch. Its read-only preflight checks
  publication serial and generation admission before aggregate runtime capture.
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

Macro argument regions own compact per-parameter ranges in pooled traced
spelling or a sparse lane of actual retained definition/region sources.
`src/input/arguments/words.rs` owns those ranges and spelling views;
`backing.rs` owns source retention and actual fragmented-argument materialization.
The same matcher writes both actual backings. Parameter rows use local offsets;
source aliases flatten once at capture. Native release visits only sparse actual
leases and walks its reserved free-list suffix iteratively. Normal ranges have
no destructor. Active captures and macro regions remain excluded from full
checkpoints and formats.

The live scanner owns `ScannerStatus` directly. `src/processor/status.rs` owns
its actual diagnostic targets and scoped prior/installed status; recovery copies
that status before clearing it. Synthetic warning serials, `ScannerState`,
`RecoveryContext`, and unreachable warning-only checkpoint rejection are deleted.
Alignment keeps its real partial builder, while definition/absorption statuses
carry no unused builder identities.

- `src/processor/expand/main_control.rs`: the remaining character-boundary
  consumer used by the legacy character-admission tests. Production main
  control uses the synchronous native reader in `scalar.rs`; the obsolete
  `main_control_until` entry, `MainControlWords` adapter and expanded-reader
  terminal eligibility hook are deleted. Lexical delivery tests exercise the
  shared reader's actual record consumer directly, while native continuation
  tests use the production callback and demand no command records.

Scalar scans retain only the actual typed value and diagnostic primary origin.
There is no recovery enum or pass-through recovery bookkeeping in integer,
dimension, unit, stretch or shrink results. Recovery reports and canonical zero
values are produced at their consuming scan; optional-space and restricted-range
semantics retain their actual decisions.

Keyword and optional-equals scanners return only a boolean, with no scalar
packet or primary-origin tracking. Actual keyword prefix backup and scanner
error chronology retain their existing spelling owners.

Named glue parameters scan optional equals and their actual glue specification in the
reader-owned execution arm. The glue-parameter assignment packet and scanner
adapter are deleted; the selected packed meaning already supplies the target.

Token-list assignment targets are scanned in their consuming native arm.
`src/scanners/token_list.rs` scans the shared optional-equals/value grammar
and returns actual collected or retained owners. Assignment packets and
redundant pointer-presence bookkeeping are absent. Macro definition products
retain only their checked target and published definition.

Token-list RHS selection consumes native expanded words through the shared
reader. Ordinary probes construct no `CurrentCommand`; actual exceptional
backup metadata stays with its existing delivery owner.

The existing `consume_expanded_words` scanner seam lends native expanded words
to executor-owned arithmetic target grammar. Invalid-target printing constructs
`PrintCommand` from the demanded native meaning and control-sequence identity;
ordinary arithmetic selection constructs no delivered command.

Definition target scanning uses the canonical raw spelling consumer and the
actual control-sequence or active-character identity. Its cached target
projection enum, command-only non-space fetch wrapper and test-only metadata
clearer are deleted. Malformed and frozen targets retain section 1215's exact
backup, insertion and report order, including exceptional delivery metadata.

Scanners expose actual direct `Result` entry points. The custom retained scalar
result enum and conversions, identity detach function and `_retained` scanner
forwarders are deleted. Actual scratch/filename unwinding remains in its owning
scanner. Restricted register bounds and recovery remain unchanged.

Expression operator and factor lookahead and raw/expanded math-shift probes consume native meanings in the shared reader. They back rejected spelling immediately with genuine exceptional delivery metadata; command slots and decoded-meaning helpers are absent from these probes. `src/scanners/expression/arithmetic.rs` owns factor/term/component arithmetic; the parent module owns scanner control and expression frames.

Canonical backup mutation accepts actual storage, treatment and alignment adjustment without an observation closure. Cold observation follows insertion and reconstructs previous brace alignment from the applied unit adjustment; ordinary backup carries no discarded previous-state value.

Filename grammar consumes native character meanings without per-character commands. Its one actual scanner retains Web2C tex.ch [29.526] preliminary backup, filename guard, quotation/group grammar, primary origin, string-pool publication and scalar-error unwind; redundant inner/leading/character result forwarding is deleted.

Futurelet retains its two operands in its own scanner grammar. Ordinary let scans its actual native meaning without the futurelet backup frame; target scanning and canonical backup mutation remain shared. The boolean-selected scanner and outlined RHS forwarding helper are deleted.

Restricted integer scans return the existing scalar value and genuine primary origin. The separate restricted result and discarded scanned/recovered fields in math, character-definition and stream requests are removed. Canonical bounds/recovery stay central; ifeof demands its extra observer diagnostic locally. Profile register scanning selects the real bound before the direct integer scan, without traversing another register forwarding entry.

The private HotToken wrapper is deleted. Remaining CurrentCommand consumers hold only their actual spelling, demanded delivery facts and native meaning directly; ordinary native input constructs no delivered record.

Execution-owned character boundaries expand native macros and primitives in the existing reader back edge. Lexical callers select raw boundaries as a static property of their actual consumer. The shared expanded invocation records and restores checkpoint quiescence once at its real return; per-word expansion adds no command or bookkeeping record. Exceptional and observed boundaries retain their actual delivered command.

Active mathcode treatment consumes its synthetic native spelling and backs the actual expanded word in src/processor/expand/scalar.rs, without charging the synthetic token or constructing ordinary commands. Genuine observed, undefined, end-template and alignment treatment retains its required delivery facts. The unused CurrentCommand::resolve convenience constructor is deleted; fixtures use the actual delivery constructor.

InputLevel holds its native packed frame and one InputStorage union directly. The outer source/resident union, SourceLevel, ResidentTokenRow, ResidentTokenStorage, ResidentSourceTop and stored_common accessors are deleted. SourceSlot owns variable source state and borrows the real frame for advancement; common identity, source context and token behavior/retirement project from the authoritative level. Native checkpoints copy the actual frame and source/list owners, excluding active macro bodies and arguments. No header adapter, substitute row wrapper or compatibility type remains.

Diagnostic input coordinates project the existing next-level identity for push/pop validity. The duplicate row_admissions field and hot increment are deleted. InputStack's context epoch changes only for checkpoint/candidate replacement and source-owner rebind; actual top/scalar facts and that cold epoch preserve stale diagnostic rejection.

The duplicate CommandClass enum and its CurrentCommand class projection are deleted. Noexpand dispatches directly on MeaningWord and its existing static class; protected replacement handling consumes the macro flags directly.

Math fields and delimiter operands consume native expanded words directly. Structural delimiter boundaries share the ordinary non-radical grammar; their duplicate hot nonblank fetch loop is deleted. Ordinary scalar cases construct no command record; braced fields, active restart, rejected-delimiter backup and demanded diagnostics retain canonical chronology.

The mandatory brace scanner consumes native expanded words and returns only the accepted origin or inserted-brace decision. ScannedLeftBrace<G>, its returned CurrentCommand, the unused raw branch and raw/expanded boolean selector are deleted. Real brace origins, EOF failure, backed-up recovery context and synthetic alignment accounting remain distinct.

The replacement interpreter consumes `consume_main_control` and
`consume_main_loop_lookahead` directly. Native backup and diagnostic capture
borrow actual spelling, meaning and optional exceptional delivery facts.
`recover_missing_math_shift` takes that spelling and exceptional record instead
of requiring a newly constructed ordinary command. There is no overload for
the deleted command-valued signature.

Immediate I/O selection is executor-owned native terminal dispatch. `write.rs`
retains the real write-text scan and sentinel expansion; the former
`ImmediateExtension` request union and command reconstruction scanner are deleted.

Native scalar and main-control consumers use concrete synchronous callbacks.
`consume_main_source_characters` lends the actual source suffix to the live
horizontal builder with explicit operation and fuel bounds. It shares the
canonical source cursor and constructs no ordinary command or token list.
Alignment endv proves its exhausted input independently of §791 cell closure,
so executor off-save recovery can retain a cell while closing a nested group.
Brace resets belong to real column and row lookahead rather than cell finish.

Native terminal consumers use statically inlined callbacks in the canonical
reader. Integer-parameter queries read the existing admitted dense bank directly;
there is no pass-through untracked-integer alias or tracing cache to synchronize.

Command and conditional trace printers query the executor's live mode prefix
at the actual print and advance its existing `shown_mode` owner there (TeX82
§299/§367, with §1370 no-mode during deferred writes). There is no owned prefix
mailbox, trace-printed flag or tracing preparation before a fetch.
