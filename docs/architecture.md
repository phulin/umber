# Engine Architecture — Core Subsystems

Status: draft companion to [core_state.md](core_state.md)
Scope: everything *around* the state layer — the processing pipeline
(input → lexing → expansion → execution → typesetting → page building →
shipout), the service layers (fonts, output drivers), and the advanced
consumers (incremental engine, JIT). The state layer itself (`tex-state`)
is specified in `core_state.md` and treated here as a given.

How to read this document: it is a normative design spec with
implementation status woven in. Passages marked **Status:** (and sentences
phrased "the implemented …" / "currently …") describe what exists today
and change as milestones land; everything else states the architecture the
implementation must converge to. File-level detail about each crate lives
in that crate's `AGENTS.md`.

---

## 1. The big picture

Knuth describes TeX as a digestive tract — eyes, mouth, gullet, stomach —
plus the typesetting routines behind it. We keep that pipeline shape (it is
the correct factoring of TeX's semantics), but reorganize it around one
architectural commitment inherited from the state layer:

> **Every subsystem is a pure-ish function over `Universe`.** All semantic
> mutation flows through the barriered state API; all external effects flow
> through `World`; all content is immutable once frozen. A subsystem's
> behavior is therefore fully determined by (input position, `Universe`
> state) — which is what makes snapshots, memoization, and the
> JIT *queries and consumers* rather than invasive rewrites.

```text
                              ┌────────────────────────────────────────────────┐
                              │              INCREMENTAL ENGINE                │
                              │  convergence · pure-kernel memoization         │
                              │  (drives everything below via snapshots)       │
                              └───────────────┬────────────────────────────────┘
                                              │ snapshot / rollback / fork
                                              ▼
   bytes           tokens            expanded tokens        nodes           pages
┌────────┐      ┌─────────┐      ┌───────────────────┐  ┌───────────┐  ┌──────────┐
│ INPUT  │ ───▶ │  LEXER  │ ───▶ │  EXPANSION        │─▶│ EXECUTION │─▶│  PAGE    │
│ layer  │      │ (eyes)  │      │  (gullet)         │  │ (stomach) │  │ BUILDER  │
└────────┘      └─────────┘      └───────────────────┘  └─────┬─────┘  └────┬─────┘
    ▲                │  ▲            │        ▲               │             │ shipout
    │                │  │ catcode    │        │ meanings      ▼             ▼ (commit)
    │                │  │ gens       │        │          ┌───────────┐  ┌──────────┐
    │                ▼  │            ▼        │          │ TYPESET   │  │ OUTPUT   │
    │           ┌────────────────────────────────────┐   │ kernels   │  │ DRIVERS  │
    │           │      tex-state  (Universe)         │   │ par·math· │  │ PDF/DVI/ │
    │           │  Env · tokens · nodes · journal ·  │◀──│ align     │  │ HTML     │
    │           │  code tables · effect log · World  │   └─────┬─────┘  └────┬─────┘
    │           └────────────────────────────────────┘         │             │
    │                        ▲         │                       ▼             │
    │                        │         │ effects          ┌───────────┐      │
    └── file reads via World ┘         └─────────────────▶│  FONTS &  │◀─────┘
        (content-addressed)                               │  METRICS  │
                                                          └───────────┘
```

Two flows to keep distinct when reading this document:

- **The token flow** (left to right above): bytes become tokens become
  nodes become pages. This is the classic pipeline; it is *demand-driven*
  (the stomach pulls from the gullet pulls from the mouth pulls from the
  eyes) because TeX's semantics require it — expansion can change catcodes,
  which changes lexing of text not yet read.
- **The state flow** (everything touching the box at the bottom): every
  stage reads meanings/codes from `Universe` and writes through the barrier.
  Mutable state that crosses an engine checkpoint boundary is rooted in the
  checkpoint tuple (input stack summary, mode nest, stream buffers — see
  `core_state.md` §9). Recursive scanners may keep ordinary Rust locals during
  their dynamic extent, but they cannot emit an engine checkpoint.

The concrete ownership boundary is `EngineCheckpoint`.  It is an engine-level
composition over an opaque `UniverseSnapshot`, the live `InputStack` root,
the executor `ModeNest` root, a named safe-boundary kind, and the retained
effect boundary. Pipeline crates continue to
own their algorithms and compact rooted representations; `tex-state` continues
to own all live handles, mutation, validation, `World`, and atomic store/world
rollback.  The driver or a future `tex-engine` facade is the only layer allowed
to capture or restore that composition, and it must synchronize the live input
cursor and mode root immediately before doing so.

Checkpoint fields follow the three-bucket contract in `core_state.md` §9.1:
TeX-semantic state, boundary-owned execution state, and discardable
derived/diagnostic state. The first two buckets determine future behavior and
semantic hashes; the third is recomputed after restore and is excluded from
equality, convergence, and durable formats. An `EngineCheckpoint` is
restartable by construction and can be emitted only by the outer executor at
an approved boundary. V1 boundaries are job start, eligible outer-paragraph
completion, and outermost shipout/output completion. Display-math completion
is an evidence-gated extension; inline-math completion is deliberately not a
v1 boundary. Format images are a separate versioned DTO for validated
quiescent state, not serialized engine checkpoints.

`retained_group_roots.md` defines the proposed extension for ordinary grouped
prose. It preserves the `Env`/`Stores`/`Universe` authority boundaries and does
not make nested scanners, boxes, insertions, alignments, or math builders
restartable.

## 2. Crate map

The workspace as it exists today (children are dependencies; the pipeline
crates also sit on `tex-state`, except `tex-out` and `tex-fonts`, which
must stay below it):

```text
umber (CLI / driver — composes the pipeline crates directly today)
 └── tex-exec        stomach: mode machine, unexpandable prims, main-control
      │              dispatch; snapshots execution parameters for pure kernels
      ├── tex-expand     gullet: macros, conditionals, expandable prims
      │    └── tex-lex       eyes+mouth: decoding, catcodes, input stack
      ├── tex-typeset    pure kernels: par builder, line break, math, packing
      │                  — consumes immutable state views and plain params
      ├── tex-fonts      font loading, immutable metrics
      └── tex-out        page artifact model + DVI driver (later PDF/HTML)

tex-state    the state layer (core_state.md) — substrate under everything
             above except tex-out and tex-fonts; itself depends on tex-fonts
             only for the immutable loaded-font record type (never the reverse)
tex-arith    shared scaled-point/TFM arithmetic under tex-state, tex-fonts,
             tex-out, and tex-typeset; depends on nothing
```

Support crates outside the pipeline: `test-support` (fixture and parity-test
helpers, a dev-dependency only), `corpus-manifest` (dependency-free manifest
parsing for host-side parity tooling), plus the `tools/` and `benchmarks/`
crates described in the root `AGENTS.md`.

Planned crates, not yet in the workspace:

- `tex-incr` (P6, rides on M4): incremental engine — named-boundary
  convergence and pure-kernel memoization (§11).
- `tex-jit` (M5): the privileged consumer of the sealed `tex-state::layout`
  module (§12).
- `tex-engine`: a possible interpreter-facade crate between the driver and
  the pipeline crates; today `umber` plays that role directly.

Dependency rules (the effect rules are enforced in CI via the clippy
`disallowed-methods` list; the crate-graph rules by the workspace
manifests and review):

- Only `tex-state` owns mutation and history. No other crate defines a
  store, a cache keyed by mutable state, or interior mutability.
- Only `tex-fonts` and `tex-out` may hold large immutable resources
  (font binaries, ICU tables); they are loaded through `World` so inputs
  stay content-addressed.
- `tex-jit`, when it lands, is the sole consumer of the sealed `layout`
  module.
- Nothing will depend on `tex-incr`; it depends on everything.
  Incrementality is a *driver strategy*, not a property scattered through
  the pipeline.

---

## 3. Input layer (the eyes' outer half)

Responsibility: turn the outside world into a stable, replayable byte/char
supply.

- **Sources**: files (`\input`, `\openin`), terminal, `\scantokens`
  pseudo-files, and editor-supplied buffers (the incremental case). All
  file access goes through `World`; every read is content-addressed and
  recorded so a snapshot pins exactly what it consumed (`core_state.md` §8).
  The concrete file source used by `tex-lex` is `WorldInput`, built from a
  `World::read_file`/`\openin` `FileContent`; `tex-lex` itself owns line
  normalization and frame state, not host file handles.
  The CLI driver's TeX input policy follows TeX82's `start_input` ordering:
  after adding the default `.tex` extension, it probes the principal input's
  directory first and, only for names without an explicit directory, probes
  the ordered directories in `TEXINPUTS`. Empty `TEXINPUTS` elements are
  ignored rather than acquiring an implicit host-dependent default. Every
  probe uses the narrow `InputReadState` capability, so the successful path
  and bytes become the ordinary content-addressed `World` input record and
  require no additional checkpointed engine state.
- **Decoding**: UTF-8 native. Invalid UTF-8 is rejected before tokenization
  with its exact half-open physical byte range; no lossy conversion is used.
  Legacy 8-bit input is a future per-source lossless decoder selected up
  front, not a per-character branch in the lexer.
- **The input stack** is the one piece of pipeline-owned state the snapshot
  must summarize (`InputSummary`): a vector of frames, each either a
  *source frame* (source id + physical line/content/terminator byte ranges +
  line/column + current normalized UTF-8 line + canonical in-line byte cursor
  + synthetic-end anchor + lexer state + pending
  traced synthetic tokens + its opaque registered-source capability) or a *token-list frame* (`TokenListId` +
  `OriginListId` + index + replay kind: macro body, `\everypar`, mark, ...).
  Macro-body token-list frames additionally carry up to nine frozen argument
  token-list/origin-list pairs; replaying a
  `Param(slot)` token pushes the corresponding argument list as a nested
  macro-argument frame while the replacement body id remains unchanged.
  Open conditionals are summarized as condition frames in the same vector,
  not as expansion-owned side state. A condition frame records a stable
  live-frame identity corresponding to TeX82's `cond_ptr`, whether it is a
  regular `\if...` or `\ifcase`, the current limb (`\if`, `\or`, or
  `\else`), whether the condition is still evaluating its operands, whether
  the current and any previous limb has been taken, the `\ifcase` `\or`
  count, and the nested conditional depth observed during skip/resume scanning.
  Source reopen identity is owned by the `World` input record in the outer
  snapshot: each file-backed source frame carries its explicit `InputRecordId`,
  a generation-tagged runtime capability which pins file/editor content by
  content hash, reopens that exact source, then applies the lexer-owned
  source-frame summary. Rollback advances the World record generation before
  a discarded dense slot is reused, and cloned timelines reject each other's
  post-fork records. The record id is not inferred from the source-frame
  ordinal: auxiliary reads such as TFM loads share the `World` input log but
  never become text input frames.
  `Universe` recursively validates this complete aggregate before replacing
  its published summary; failed validation is atomic. The registered-source
  capability guards rollback reuse but does not participate in semantic
  equality or hashing.
  `last_source_frame` is also summarized with its source id so snapshots taken
  just after a source pops still have final source coordinates for EOF/current
  input diagnostics. The summary separately retains the next-source-id
  allocator high-water mark and Unicode `^^` mode, preventing resumed input
  from reusing a live id or silently changing lexer configuration.
- Line-oriented details TeX cares about (`\endlinechar`, trailing-space
  trimming, `%` line ends) live here, driven by parameters read through the
  aggregate state API. **Status:** the implemented `tex-lex` input layer
  exposes a local `InputSource` trait for memory buffers and files. Each input
  line retains its original backing-byte start, content end, and LF/CRLF
  terminator range; normalization trims trailing spaces and appends the current
  `\endlinechar` when valid, including for blank/all-space physical lines. A
  `SourceFrame` stores the normalized line as UTF-8 and advances one canonical
  byte cursor plus the separately required scalar column, so source-coordinate
  production is O(1) and lookahead/rewind restores both values. It owns an
  `InputStack` whose `InputSummary` records resume-complete lexer-owned
  source frame state and token-list replay progress. Mutable input-stack
  delivery carries `TracedTokenWord`; plain-`Token` methods are compatibility
  shims that decode and drop origins. Token-list frames read frozen content only
  through `Universe::tokens` and pair it with the frame's origin-list span when
  one exists. Stored non-macro token lists without an origin-list home
  (`\toks`, `\everypar`, marks, output, writes) replay with synthetic
  per-replay-kind origins in v1; durable source identity is
  the explicit `World` input record captured on source origins and in
  `Universe` snapshots, which pins file bytes by content hash so a driver can
  reopen the exact source and apply the lexer summary. `\endinput` is
  represented as a source-frame flag that lets
  the lexer finish the current normalized line and then pop that source
  without asking expansion to manage source internals.
  **Source-map status:** built-in World and memory sources expose immutable
  descriptors and are idempotently registered through the narrow
  `ExpansionState` capability before their first traced delivery (including
  empty sources). `Universe` validates World record liveness/length before
  the private source map accepts a region; file bytes remain solely in
  `World`, while generated sources share immutable backing with their input
  adapter. Registration yields an opaque `RegisteredSource` capability held
  only by the live source frame, so ordinary backed one-scalar tokens encode
  their logical position directly without a repeated map lookup or a
  provenance-arena append. Positions above the direct payload use
  validated arena `SourceSpan` records. Control sequences, transformed `^^`
  input, and other multi-character spellings use exact validated half-open
  spans; inserted normalized endlines use zero-width physical anchors. Phase 6
  measurements adopt the tagged form: ASCII/UTF-8 logical bytes fall by more
  than 93%, source-heavy throughput improves, and no primary workload regresses
  more than 5%. Flat source records remain only as degraded compatibility for
  explicitly unregistered legacy/test origins; production traced inputs do not
  emit them.
  Diagnostic resolution dispatches all forms
  through the live source map and computes physical line/column data lazily,
  so frame pop does not lose source text and aggregate rollback cannot alias
  reused ids. Replay reads origin lists through a best-effort liveness query:
  a stale diagnostic side table degrades to unknown while its independently
  live semantic token list continues to execute.

## 4. Lexer (the eyes)

Responsibility: characters → tokens, under mutable catcode law.

- **Semantic core**: the classic catcode state machine (states N/M/S,
  control-sequence scanning, `^^` notation, catcode pairs from the code
  tables). This is the reference implementation and the arbiter.
- **Fast path**: a SIMD classifier over whole buffers that assumes the
  current catcode *generation vector* (`core_state.md` §5). It
  pre-tokenizes runs of "boring" text (letters, spaces, `\par` boundaries)
  in bulk. Any generation bump — someone wrote a catcode — invalidates the
  precomputed classification from the write position forward; the machine
  falls back to the semantic core and re-warms.
- **Speculation discipline**: the fast path may run *ahead* of execution
  (pre-lexing the rest of the buffer) precisely because tokens it produces
  carry the generation vector they were lexed under; consuming a stale
  token is impossible, only wasteful.
- Control-sequence identities intern immediately through the permanent
  process-wide name registry to `Symbol`; named sequences
  and active characters occupy distinct interner namespaces even when their
  printable spelling is identical. The semantic token
  type remains `Token = Char(char, Catcode) | Cs(Symbol) | Param(u8)` — one
  word, `Copy`. Hot token movement uses `TracedTokenWord(u64)` beside it:
  bits 63..62 are token kind, bits 61..32 are a 30-bit payload, and bits
  31..0 are `OriginId`. Character payloads store a 21-bit Unicode scalar value
  plus 4-bit catcode, control-sequence payloads store `Symbol::raw()`, and
  parameter payloads store the 4-bit slot. The opaque origin field privately
  reserves zero for Unknown/Bootstrap, uses clear-high-bit payloads for direct
  logical source positions, and uses high-bit payloads for provenance-arena
  indexes. Encoding capacity never narrows the logical `u64` source space;
  allocation overflow saturates to unknown rather than aborting semantic
  compilation. Origin records and packed
  origin-list spans live in `tex-state` as rollback-coupled, hash-neutral
  diagnostic side-channel arenas. User-facing source labels, line/caret
  snippets, and expansion traces are rendered lazily by the provenance
  resolver at diagnostic formatting boundaries. Errors capture a bounded
  primary/related `DiagnosticSite` plus one macro-invocation chain head before
  replay frames pop;
  paths, excerpts, line indexes, and Unicode/tab display widths remain lazy.
  One parent-linked invocation origin is shared by each macro replay frame.
  The input stack maintains its active head in O(1); one innermost retired head
  is retained for the current delivery attempt so EOF and pre-token errors keep
  the complete chain without leaking it to a later unrelated token. The
  resolver applies the requested presentation depth. Macro-body delivery reuses
  frozen origin lists and performs no provenance write per delivered token.
  Scanner range composition requires lexer-issued proof of two ordered direct
  deliveries from the same still-live physical frame, so replayed or expanded
  endpoints cannot be made contiguous from origin ids alone. Hot token
  movement never formats strings for provenance.
- The lexer holds **no state outside the input stack frame** (its N/M/S
  state is part of the frame). Nothing here needs journaling.

## 5. Expansion engine (the gullet)

Responsibility: the token-level rewriting system — macros, conditionals,
`\expandafter`, `\csname`, `\the`, `\number`, e-TeX expandables.

- **Structure**: a `get_x_token` loop. Pull a traced token word; look up its
  semantic token's meaning word in `Env` (one load); if expandable, push its
  expansion as a token-list frame and continue; else deliver the same
  `TracedTokenWord` downstream. Control-sequence tokens address their interned
  symbol directly; active character tokens address the typed active-character
  symbol used by definition assignments, distinct from an escaped
  one-character control symbol with the same spelling. Compatibility callers that still need
  plain `Token` values decode only at their boundary and do not fabricate
  replacement origins.
  Undefined control sequences follow TeX82's expansion path: `get_x_token`
  reports an expansion error and forgets the consumed token rather than
  delivering it as an unexpandable command.
- **Macro call**: match delimited/undelimited parameters against the
  incoming stream (argument scanning is the gullet's inner loop and the
  #1 profile target); arguments are built with token builders and frozen.
  Delimited argument scanning follows TeX82's partial delimiter recovery:
  for non-`long` macros, a `\par` token that was tentatively matched as part
  of a delimiter prefix is allowed if that prefix later fails and is recovered
  into the argument, while ordinary argument `\par` tokens still abort the
  call.
  Macro meanings decode through the state aggregate into flags plus frozen
  parameter-text and replacement-text token-list ids. The body replays as a
  frame with argument slots resolved by index — no token-list copying for
  the body, ever (bodies are shared `TokenListId`s).
- **Macro definition scanning**: `tex-expand` exposes the shared
  `scan_toks`-style scanner for `\def`/`\edef` syntax. It scans parameter
  text and a brace-balanced replacement body into frozen token/origin lists,
  including TeX's ordered `#1`..`#9` parameter markers, trailing `#{`, and
  doubled `##` replacement-body escapes. The scanner may freeze content
  through `Universe`, but it does not assign meanings; the stomach/future
  assignment layer remains responsible for installing the returned
  `MacroMeaning`.
- **Numeric value scanning**: `tex-expand::scan_int`,
  `tex-expand::scan_dimen`, and `tex-expand::scan_glue` own the reusable
  expanded integer, dimension, glue, and muglue scanners for conditionals and
  the later stomach assignment layer. They pull
  through `get_x_token`, except that the integer scanner raw-reads the single
  operand following a backtick character constant. The integer scanner
  understands TeX integer constants and currently readable integer-like state (`\count`, chardef
  values, `\endlinechar`, and raw-sp `\dimen` coercion), while the dimension
  scanner parses decimal constants, physical units, `true` units, supported
  internal dimensions (including named dimension parameters, glue-parameter
  widths coerced to dimensions, and decimal factors applied to internal units),
  `mu` dimensions for muglue callers, infinite `fil`
  orders for glue components, and opt-in integer-to-sp coercion. The glue
  scanner parses optional `plus`/`minus` components and interns immutable glue
  specs through `Universe`. These scanners report recoverable numeric diagnostics
  without performing assignments; missing numbers recover as zero, missing or
  incompatible finite units recover as inserted `pt` or `mu` according to the
  caller, and glue scanning keeps scanning `plus`/`minus` components after those
  recovered values. Execution-side assignment callers route all scanner
  diagnostics through TeX's terminal/log diagnostic path before applying the
  recovered value. Scanner pushback of already-read tokens preserves the
  original traced token origins instead of wrapping them in fresh unread
  provenance, so diagnostics can continue to point at the source token that
  caused the scan decision. Like TeX82's command-code gate before
  `scan_something_internal`, ordinary unexpandable commands in a numeric slot
  recover as a missing zero and remain available for stomach execution; only
  meanings classified as internal numeric quantities enter internal-value
  decoding. Scanner errors and recoverable scanner diagnostics
  attach primary origins from the offending token; when a scanner fails at end
  of input, the origin is allocated from the current source frame or the
  retained `last_source_frame` coordinates. `true` physical units call the
  `Universe::prepare_mag` boundary before scaling, so illegal magnifications
  are coerced and the job-level magnification is frozen in snapshot-covered
  state for later shipout/font paths. Font-relative `em`/`ex` units remain
  explicit TODO stubs until font metrics exist (umber2-flt).
- **Conditionals** are a frame-kind, not a side stack: `\if...` evaluation
  marks the frame; `\else`/`\fi` skipping is a token-level scan that the
  fast lexer can accelerate (skip mode only needs catcode classes for
  `\`-detection). The condition stack is part of `InputSummary` and carries
  limb/taken state, an operand-evaluation bit matching TeX82's `if_limit`,
  `\ifcase` `\or` count, and skip nesting so rollback can restore an open
  conditional without reconstructing hidden gullet state.
- **`\csname`** interns through the same interner; **`\the`/`\showthe`**
  read `Env` and mint fresh frozen token/origin lists.
- **`\noexpand`** preserves the suppressed control-sequence token and its
  provenance for token-identity consumers, while its checkpointed replay kind
  and inserted origin make an expandable meaning behave as `\relax` for that
  single main-control delivery. TeX82 leaves an already-unexpandable meaning
  unchanged, so a suppressed `\cr` or `\span` still terminates an alignment
  cell normally.
- **Alignment delivery state** lives with the active `tex-lex` alignment
  input, not with stomach group depth. It spans `scan_spec`, preamble scanning,
  row peeking, and cell template replay just like TeX82's global `align_state`,
  and is saved wholesale while a nested alignment runs. `tex-expand`
  classifies literal character braces at each `get_next`-style delivery;
  control sequences let to brace meanings retain their execution command but
  do not take the character-token scanner branch. An
  expandable noexpand-suppressed meaning classifies as ordinary and an
  unexpandable suppressed meaning retains its brace/delimiter class. Scanner back-input uses one
  boundary that reverses only a transition recorded for an actually delivered
  token before replay; synthetic insertion therefore cannot perturb the
  sentinel. The preamble starts at `-1000000`, row peeking resets to `1000000`,
  and `fin_col` restores that sentinel before fetching the first token of every
  continuing column. U-template retirement then resets to zero before the first
  body token is counted, and only a delimiter delivered at zero starts the
  v-template. This is local alignment state, not an engine checkpoint.
- **Read-set recording** hooks live here and in the stomach: when the
  incremental engine asks for it, meaning lookups record `(cell, epoch)`
  pairs (`core_state.md` §9). Off by default, zero-cost when off (the
  recorder is a generic parameter of the loop, monomorphized away).
- **Status:** the implemented `tex-expand` scaffold exposes that loop over
  `tex-lex::InputStack` through the shared `ExpansionState` capability, not
  broad `&mut Universe`. Production callers wrap the owning `Universe` in
  `ExpansionContext` before entering the gullet. That capability allows meaning
  reads, immutable token/glue/font/node/register/parameter reads, token-list
  freezing, glue interning, magnification preparation, lexer control-sequence
  interning, and `\csname`'s relaxed control-sequence interning. `ExpansionState`
  cannot construct input-read authority; the top-level expansion/dispatch path
  additionally carries `InputOpenState` only so `\input` can create an
  `InputOpenContext`. Scanner and helper recursion does not receive that
  authority directly. Instead recursive traced expanded-token reads go through the
  narrow `ExpandNext` capability; the top-level driver supplies a
  `DriverExpandNext` implementation that can re-enter dispatch with `\input`
  authority, while ordinary helper-only paths use a no-input implementation.
  Dimension, glue, condition-token, register-index, and `\the` operand scans
  therefore expose both no-input helper entry points and explicit
  expander/driver-aware entry points for production callers that already own
  input-read authority.
  File reads for `\input` live behind the separate `InputReadState`
  capability; the concrete `ExpansionContext` holds an object-safe
  `InputResolver`, which receives an `InputOpenContext`, not `ExpansionState`,
  only when dispatch actually executes `\input`. The resolver can open input
  files without seeing meaning reads, Env/register
  writes, code-table writes, grouping, snapshot, font-assignment, or general
  World mutation APIs. Macro body replay uses
  the body `TokenListId` directly plus its definition-time `OriginListId`, a
  one-per-call macro-invocation origin, and frozen argument token/origin pairs
  on the replay frame; it does not allocate a substituted body list. Diagnostic
  expansion backtraces are reconstructed from those live replay-frame
  invocation origins with a fixed display depth, not from per-token chain
  records. Token-list replay is
  naturally read-only; source-frame replay may intern newly encountered
  control sequence names through the lexer/interner capability. `\csname` uses a dedicated
  expansion scan that stops on `\endcsname`, accumulates only expanded character
  tokens, and interns/relaxes the resulting control sequence through the named
  namespace of the same aggregate boundary. Its synthesized control-sequence token is replayed through
  the ordinary `get_x_token` loop, so a macro result expands before execution
  sees a token and its synthesized origin remains the macro invocation parent.
  If expansion yields a non-character token before
  `\endcsname`, the scanner follows TeX82 recovery by treating a missing
  `\endcsname` as inserted and pushing the offending token back through an
  `Inserted(Unread)` replay frame. Value-producing expandables such as
  `\number`, `\romannumeral`, `\string`, `\meaning`, `\the`, `\fontname`,
  and `\csname` allocate one shared `Synthesized` origin for each generated
  run, parented by the primitive token that caused the run. Primitive installation and
  stomach assignment/test setup helpers still receive `&mut Universe`, but
  the production token-reading and scanner path is Rust-enforced against
  Env/register/code-table writes.
- Frame-control expandables are represented as input-frame rewrites:
  `\expandafter` saves one raw token, performs one expansion step on the
  following token, then pushes the saved token above the expansion result with
  an `Inserted(ExpandAfter)` origin; `\noexpand` delivers or replays the next
  token with an `Inserted(NoExpand)` origin and suppresses expansion for
  exactly the next `get_x_token` read. This keeps suppression frame-local and
  avoids mutating `Env`.
- **Status:** implemented conditional predicates evaluate in `tex-expand` and
  record their result by pushing/updating `tex-lex` condition frames. `\if` and `\ifcat`
  expand only to the two unexpandable comparison tokens; `\ifx` reads two raw
  tokens and compares macro meanings by flags plus semantic
  parameter/replacement token-list contents, with non-macro control sequences
  falling back to meaning-word equality. `\ifnum`, `\ifdim`, `\ifodd`, and `\ifcase` reuse the
  shared integer/dimension scanners, including `\ifcase` `\or` limb selection.
  Mode and last-item predicates read ordinary fields from the
  `EngineStateSnapshot` stored in `ExpansionContext`; box predicates read only
  the `Universe` box-register facade; `\ifeof` reads the `World` input stream
  table directly through `ExpansionState`. False
  conditional limbs and already-taken `\ifcase` limbs are
  skipped by reading raw tokens from `tex-lex` under the active catcode table,
  while `\else`, `\or`, and `\fi` update the input-stack condition frame and
  report extra-control, incomplete-conditional, and skipped-outer-token
  diagnostics. A delimiter expanded while the current condition is still
  evaluating its operands is handled like TeX.web's `insert_relax`: the
  delimiter is pushed back and a relaxed token is inserted ahead of it.
- **Superscript notation** is normalized before catcode-driven tokenization
  using the current superscript catcode, including TeX's chained case where
  one doubled-character escape decodes to another superscript character that
  immediately combines with the following input (for example the TRIP
  `qq5e^5c` construction after `q` becomes superscript).
- Value-rendering expandables (`\string`, `\number`, `\romannumeral`,
  `\meaning`, and the currently supported `\the` classes) mint their visible
  output through the explicit token-list freezing capability. `\the` covers
  integer, dimension, glue, muglue, and token registers; register aliases;
  integer, dimension, glue, muglue, and token parameters represented in `Env`;
  and code-table values. Named math glue parameters such as `\thinmuskip`,
  `\medmuskip`, and `\thickmuskip` are exposed as muglue assignment targets
  while their values remain in the glue-parameter bank at TeX's parameter
  indices. Font dimensions, box dimensions, page state, and
  Font dimensions use TeX82's `scan_font_ident` forms, including `\font` for
  the current font, and validate reads against the snapshot-covered Env-side
  parameter count before rendering the exact scaled value. The same scanner
  resolves `\textfont`, `\scriptfont`, and `\scriptscriptfont` through the
  barriered three-by-sixteen family bank; `\the` replays the selected font's
  immutable identifier control sequence, whose identity participates in font
  semantic hashes. Time/job parameters not yet backed by `Env` remain
  documented TODOs until those classes are semantically available.
- Input/job expandables use explicit session context: `tex-expand` scans the
  `\input` file name and invokes the context's object-safe `InputResolver` for
  a new `InputSource`, while `\jobname` renders plain session data. The
  resolver is dynamically dispatched only for `\input`; scanner and ordinary
  token dispatch carry no resolver type parameter. This preserves the rule
  that file access belongs to `World`/the driver layer, not to the gullet.
  `\fontname` renders loaded font selector names. The mark-family expandables
  replay the frozen token lists stored in the Universe-owned page mark slots;
  empty slots replay the canonical empty token list.
- **Status:** the stomach implements the macro-definition assignment surface
  used by the expansion conformance path: `\def`, `\edef`, `\gdef`, `\xdef`, `\let`,
  `\futurelet`, prefix accumulation (`\global`, `\long`, `\outer`,
  `\protected`), and `\globaldefs` override behavior. Definition targets use
  TeX's `get_r_token` rule: either a control sequence or an active character
  is accepted, with active characters stored under their typed active-character
  symbol rather than the same-spelling named control-symbol identity.
  `\let` then follows TeX82's raw-token scan: it skips spaces before an
  optional equals sign, skips at most one space after that sign, and copies the
  already-tokenized command or character meaning without expansion. Prefix
  accumulation itself follows TeX82's expanded scan: after each prefix,
  it expands macros and skips spaces and `\relax` until the command to execute
  is reached. Thus `\global` can qualify a macro whose expansion begins with an
  assignment, while the delivered assignment token retains its expansion
  provenance and read-set recording. Code-table
  assignments use the same prefix/globaldefs policy as other definitions;
  their structurally persistent roots restore local assignments at group exit.
  These commands scan through the shared gullet/token scanner where expansion
  is required and write meanings only through the barriered `Universe` facade. The
  `umber expand-dump` driver delegates those primitives to `tex-exec` before
  printing delivered tokens; its remaining local assignment handling is
  limited to dump-corpus scaffolding such as `\chardef` and `\catcode` until
  those stomach assignments land. Expansion and execution failures are
  rendered through the provenance resolver while the driver's live input and
  universe state remain available, so this diagnostic-only command reports
  source snippets and bounded macro traces like a normal run.
- **What the gullet never does semantically**: perform TeX assignments.
  `\def`, `\advance`, register writes, and code-table writes are
  *unexpandable* — they are delivered to the stomach. This is TeX's own
  factoring, and the implementation follows it behaviorally: expansion
  routines receive `ExpansionState` for reads and for sanctioned immutable
  content/interner operations only; recursive expanded-token reads from
  scanners are mediated by `ExpandNext`, so scanner signatures never expose
  file-open authority. Expanded-token entry points also carry the separate
  `InputOpenState` authority needed for `\input` dispatch. Because
  `ExpansionState` omits input-open construction and barriered assignment
  methods such as meaning, register, code-table, group, and font setters, "the
  gullet cannot mutate Env/register/code-table state" and "ordinary scanner
  helpers cannot open input files" are enforced Rust API boundaries rather than
  conventions.

## 6. Execution engine (the stomach)

Responsibility: the mode machine and every unexpandable primitive —
assignments, box building, and dispatch into the typesetting kernels.

- **Main control** is a loop over (current mode × delivered token meaning):
  vertical, horizontal, math, and their internal/restricted variants. The
  gullet delivers `TracedTokenWord` values to the stomach; execution decodes
  the semantic `Token` only for meaning dispatch and mode behavior while the
  raw `OriginId` rides alongside for diagnostics. Execution errors that are
  caused by consumed input cross main-control boundaries with an owned bounded
  `DiagnosticSite`, preserving the primary, labeled related locations, and
  invocation ids even after replay frames pop. A later diagnostic renderer
  resolves the site before rollback can invalidate its provenance records.
  The mode stack (nest) tracks the list under
  construction per level; the current list is an *unfrozen node builder* (§7
  of `core_state.md` — builder-then-freeze applies to node lists exactly as
  to token lists).
- **Assignments** are thin: decode operand, scan value (number/dimen/glue
  scanning lives here — it consumes expanded tokens), call `Env::set` /
  register setters. `\global` maps to the tagged journal write. Grouping
  primitives (`{`, `}`, `\begingroup`, `\endgroup`) map to journal
  group markers — the stomach contains **no save-stack logic**; it calls
  `universe.enter_group()` / `leave_group()` and `\aftergroup` tokens are
  the group marker's payload. Mode-independent assignments follow the same
  executor path in vertical, horizontal, and math modes; in particular,
  `\setbox` in math mode scans and builds through the ordinary box machinery
  and stores through the same barriered `Universe` box-register facade.
- The arithmetic-only substrate for dimension scanning lives in
  `tex-state::scaled` — a compatibility re-export of `tex-arith` (§9), which
  owns the implementation so state and font parsing share it without a
  dependency cycle: TeX's `xn_over_d` conversion routine, decimal fraction
  rounding, the physical-unit conversion table, and the `max_dimen` range
  check with the canonical `Dimension too large` diagnostic. Token parsing,
  signs, `true` magnification, internal units, and assignment effects remain
  scanner/stomach responsibilities.
- **Box building**: `\hbox{...}` etc. scan a packing spec and recognize the
  opening and closing brace commands by meaning, so `\let` aliases of
  begin/end-group characters delimit the box exactly like literal braces.
  The scanner enters that brace group as a normal journal-backed group, then
  executes a nested
  restricted-horizontal or internal-vertical list builder, freeze the
  finished list into the epoch arena, then call the pure `tex-typeset`
  packing kernel while the box-local assignments are still visible. The
  group is left before the resulting box is stored or appended, so only
  global assignments survive outside the builder. When horizontal packing
  reports an overfull box and
  `\overfullrule` is positive, the execution hand-off appends TeX's
  running-height rule node to the packed child list before the box can be
  stored, appended, or shipped. Storing the resulting one-node list in a box
  register is the barriered promotion write. Pulling boxes back out through
  `\copy` or `\box` pins the self-contained survivor root and appends the box
  node with its existing children. Unboxing pins the root and splices only its
  top-level children; deeper descendants remain survivor-backed. `\lastbox`
  already owns current construction material. Box-dimension rewrites copy only
  the one outer box node and reuse its child span until the register write
  promotes the replacement. Destructive unboxing validates
  the requested horizontal/vertical list kind before taking the register, so
  TeX's incompatible-list recovery preserves the register and its survivor
  ownership exactly; copy variants never clear it. In math mode, the applicable
  box command family (`\hbox`, `\vbox`, `\vtop`, `\box`, `\copy`, `\vsplit`,
  and shifted `\raise`/`\lower` boxes) uses the same scanners and packers, then
  contributes an Ord noad whose nucleus is the frozen one-box `SubBox` field;
  register extraction still goes through the same-level `Universe` facade.
  Horizontal list construction keeps one unresolved font-backed glyph per mode
  level and resolves each new adjacent character against it through the loaded
  font's TFM ligature/kern program. Resolved nodes append directly to the mode
  list; a boundary command or literal group token finalizes only the last glyph,
  so the normal execution path needs neither a pending-character vector nor a
  temporary reconstitution node vector. The run retains its first character and
  starting node offset so left-boundary output can be inserted at the correct
  position when the run is finalized, after `\noboundary` has been observed.
  In unrestricted horizontal mode, finalization also inserts TeX82's null
  discretionary after a literal character matching the current font's
  `\hyphenchar`; restricted horizontal lists suppress the insertion. Literal
  groups therefore end the current character run exactly like TeX82's
  non-character `main_loop_lookahead`, so constructs such as `{f}i` suppress
  cross-boundary ligatures. The compact unresolved-glyph state is part of the
  mode summary, preserving snapshot and replay isolation without separately
  allocated copy-on-write storage. The stomach
  updates the mode-local `\spacefactor` and appends explicit
  h-mode nodes for spaces, kerns, skips, finite-fill glue, penalties, rules,
  discretionaries, accents, and italic corrections. Text-accent horizontal
  displacement uses `tex-arith`'s widened fixed-point form of TeX82's formula
  and signed tie rounding; no semantic `real` enters the constructed kerns.
  Paragraph breaking is a
  hand-off to the pure `tex-typeset` line breaker; resulting outer-vertical
  material is appended to the Universe-owned page contribution list.
  Leader primitives scan a box or rule payload followed by mode-appropriate
  glue, then attach that payload directly to the emitted glue node; DVI
  repetition remains output-driver work, not packing-kernel state. Packing
  still treats the leader node as ordinary glue along the leader axis while
  accounting for the payload's perpendicular dimensions: horizontal leaders
  can contribute height/depth to their hbox, and vertical leaders can
  contribute width to their vbox.
  Vertical list construction tracks TeX's `prev_depth` on each mode-list
  level. A single shared append routine handles every box or rule appended to
  vertical/internal-vertical lists, including explicit box appends, unboxed
  vlist children, and paragraph lines. It inserts the implicit adjusted
  `\baselineskip` glue, or `\lineskip` when the adjusted baseline glue is
  below `\lineskiplimit`, unless `prev_depth` is TeX's ignore sentinel. This
  keeps baseline/interline side effects in the stomach boundary; `tex-typeset`
  receives explicit glue nodes and remains a pure measurement and packing
  kernel.
- **Math front-end**: `tex-exec` owns math-mode entry/exit and Appendix G
  mlist construction. It turns math characters, explicit noad constructors,
  scripts, generalized fractions, radicals, accents, `\vcenter`, style
  switches, mu glue/kerns, `\mathchoice`, and `\left...\right` groups into
  frozen `tex-state` math node payloads. A matching `\right` closes the
  nested math level and appends an inner noad whose nucleus is the delimited
  sub-mlist; delimiter sizing remains a pure conversion-time responsibility.
  Non-radical delimiter scanning follows TeX82's `scan_delimiter`: expansion
  skips blanks and `\relax`, character delimiters read `\delcode`, and the
  unexpandable `\delimiter` command scans the exact 27-bit small-family,
  small-character, large-family, large-character word. Invalid tokens are
  replayed with their original traced origin before null-delimiter recovery,
  so later diagnostics and restored input checkpoints retain the same source.
  When `\delimiter` itself appears as math material, TeX's high 15-bit
  math-character half supplies the noad class, family, and character; the low
  12-bit large variant is reserved for variable-delimiter construction.
  Closing a math-field group also applies TeX82's single unscripted-Ord brace
  simplification, exposing that atom's nucleus directly and avoiding spurious
  box nesting in grouped constructs such as Plain TeX's `\big` family.
  The mode-list summary carries the pending incomplete fraction so snapshots
  preserve TeX's `\over`/`\atop`/`\above` state.
  At formula exit, the stomach performs TeX82's `math_fonts` gate before
  conversion: all three family-2 symbol fonts must expose 22 parameters before
  all three family-3 extension fonts are checked for 13. A failure emits the
  corresponding symbol/extension diagnostic and deletes the mlist, preserving
  the surrounding math-on/math-off boundary without synthesizing an empty
  hbox; mode and math-shift group teardown then proceeds normally.
  When a `math_comp` constructor such as `\mathopen` is delivered outside
  math mode, main control follows TeX82's missing-dollar recovery: it replays
  a traced math-shift token before the original traced constructor, reports
  the diagnostic, and lets ordinary math entry and noad construction rescan
  both tokens. The inserted replay and math-entry lookahead retain the
  triggering token's origin and remain part of the checkpointed input stack.
  Superscript and subscript character tokens use the same missing-dollar
  recovery outside math mode: the original traced token is replayed after
  entering math so the math dispatcher, rather than horizontal character
  handling, scans the script. If a leaders command is then rejected where a
  script field is required, the scanner inserts a traced brace-recovery group
  around the following material, preserving both the offending token's origin
  and the recovered sub-mlist in checkpointable engine state.
  `\mathcode"8000` redispatches through the current active-character meaning
  at use time, INITEX ASCII letters/digits carry TeX82's variable-family
  mathcodes, and every inline/display formula opens a distinct journal-backed
  math-shift group before locally resetting the barriered `\fam` parameter to
  -1 and replaying `\everymath`/`\everydisplay`. Equation-number subformulae
  open a nested math-shift group with the same reset. Math conversion and
  equation-number packing occur before their respective group exits, matching
  TeX82's `after_math`/`unsave` ordering; local Env and code-table assignments
  therefore affect the formula but restore at the closing dollar, globals
  survive, and `\aftergroup` tokens replay through the checkpointed input
  stack. `\vcenter` box scanning likewise uses a journal-backed group, so
  box-local parameter changes restore when the vertical box closes while
  global assignments survive. Family font selectors live in the barriered
  Env font state.
  Display math is packaged stomach-side: entering `$$` from unrestricted
  horizontal mode interrupts the paragraph through the ordinary line breaker,
  records `\predisplaysize`, `\displaywidth`, and `\displayindent` in
  snapshot-covered display mode state, replays `\everydisplay`, and later
  appends the display hbox, optional `\eqno`/`\leqno` hbox, display skips, and
  pre/post display penalties to the enclosing vertical list while resuming the
  paragraph with TeX's `prevgraf += 3` accounting. The only pure-kernel call in
  that path remains mlist-to-hlist conversion.
  The committed math DVI corpus uses primitive-only INITEX fixtures with a
  shared include that loads the Computer Modern text, math italic, symbol, and
  extension families at 10pt/7pt/5pt; this keeps script and scriptscript metric
  differences visible without depending on `plain.tex`.
- **List diagnostics**: `\showbox` routes through `World` terminal/log
  effects and uses the shared node-list dump emitter in `tex-exec`. The
  emitter walks frozen node lists through `Universe`, honors
  `\showboxbreadth` and `\showboxdepth`, and is intentionally reusable by
  future `\showlists` and `\tracingoutput` diagnostics rather than tied to
  `\showbox` scanning. The committed typeset corpus uses pdfTeX's box-dump
  text as the comparison format, including leader glue lines followed by
  their box or rule payloads one level deeper: `umber run --show-fixtures` is an explicit
  fixture-harvesting mode whose stdout is the collected terminal/log diagnostic
  text produced by the engine run. It does not commit the pending `World`
  effect log, so stream opens, closes, and writes recorded during harvesting do
  not create unrelated host-side files. Test support still normalizes banners,
  source line echoes, and memory-irrelevant trailer noise through one shared
  diagnostic-log normalizer for both execution and box-dump fixtures.
- **Insertion building** follows the same meaning-based group boundary as box
  building: `\insert` accepts `\let` aliases of begin/end-group characters,
  preserves the traced opener while classifying its current meaning, and runs
  the internal vertical builder inside a journal-backed `Universe` group.
  Entering an `\insert` or `\vadjust` group applies TeX82's local
  `normal_paragraph` defaults, and a paragraph still open at the closing brace
  is line-broken before the internal vertical list is frozen. Starting the
  first paragraph of an empty internal vertical list omits `\parskip`, while
  later paragraphs retain it, matching TeX82's signed-mode `new_graf` test.
  Entering every `\vbox` or `\vtop` likewise applies `normal_paragraph`
  after its journal-backed group opens, so inherited `\parshape`,
  `\looseness`, `\hangindent`, and `\hangafter` values cannot shape the
  internal list and are restored when the box closes.
  Insertion parameters and content are captured before group exit, while only
  global assignments made inside the insertion survive in the enclosing state.
- **Paragraph and page hand-off**: paragraph start/end is stomach-owned.
  `\indent`, `\noindent`, implicit start from vertical-mode character
  material, `\parskip`, and `\everypar` replay are handled before entering
  unrestricted horizontal mode. Vertical-mode `\accent` follows TeX82's
  command replay path: the command is backed up before paragraph entry so
  `\everypar` runs before the accent number and base character are scanned.
  Fresh paragraphs reset the enclosing
  `\prevgraf` before line-shape selection, while display-math interruption
  retains it as the continuation offset for the resumed paragraph. When
  horizontal material ends (`\par` or
  `\endgraf`), a null unindented paragraph is popped without producing a line,
  matching TeX82's `end_graf`; otherwise the stomach performs TeX's final
  paragraph-list preparation
  (trailing-glue removal and `\penalty10000` plus `\parfillskip`), expands
  finished inline math lists into hlist nodes bracketed by `\mathsurround`
  `MathOn`/`MathOff` markers, snapshots paragraph-shape and line-breaking
  parameters, calls the pure line breaker over the prepared hlist, runs
  separate post-line-break surgery, measures each decoded line for hpack,
  inserts any overfull rule before freezing the final line once, and appends
  the hboxes through the shared
  vertical append routine. Fresh engine state follows TeX82's INITEX
  initialization: the integer and dimension banks start at zero except for
  Knuth's explicit minimum defaults (`\tolerance=10000`, `\mag=1000`,
  `\maxdeadcycles=25`, and `\hangafter=1`, plus the escape and end-line
  characters). Plain-format paragraph/layout values are established by
  loading `plain.tex`; primitive-only parity fixtures must state any format
  baseline they require. The page builder (§8) observes appends to the main
  vertical list.
- The stomach is the *only* pipeline stage holding `&mut Universe`, and it
  holds it as a plain argument — re-entrancy (e.g. `\output` routines,
  `\vsplit`-triggered mark extraction) is recursion in Rust, with the mode
  nest making it explicit. Recursive stomach operations may open scoped
  rollback transactions whose marks cannot escape the live call stack. They
  do not serialize their continuation and cannot publish engine checkpoints.
  The outer loop recognizes a boundary only after box-group scanning,
  alignment row/cell execution, `\noalign`, template replay, math construction,
  and output-routine recursion have completely unwound.
- **Status:** the implemented `tex-exec` scaffold owns that explicit mode
  nest now. Its
  summary is a vector of mode levels, each carrying one of TeX's six modes
  (vertical/internal vertical, horizontal/restricted horizontal, math/display
  math) plus the node list under construction. Main control pulls through
  `tex-expand`'s `get_x_token` loop, and the box-group scanner re-enters the
  same dispatch path for nested stomach work. That recursive main-control
  loop also reports and consumes recoverable expansion, assignment-target,
  and group-closure errors in place; it must not unwind the box construction
  transaction and expose the unread body to the enclosing list. The gullet's mode predicates
  read a snapshot of the current nest level from `ExpansionContext`, collapsing
  the six modes into the three `\ifvmode`/`\ifhmode`/`\ifmmode` families and
  the `\ifinner` bit. Box primitives are implemented for register
  round-trips, packing, unboxing, last-box extraction, dimension reads/writes,
  and shift commands. Restricted-horizontal builders also now construct
  font-backed hlist content for ordinary characters and spaces, including
  TFM ligature/kern reconstitution, space-factor glue, discretionary nodes,
  accents, rules, penalties, and italic corrections. Paragraph breaking now
  routes through `tex-typeset`; outer vertical contributions are observed by
  the page builder described in §8.

## 7. Typesetting kernels

Responsibility: the pure algorithms — node lists in, node lists out. These
are deliberately **libraries, not stages**: they own no state, do no I/O,
and read `Universe` only for parameters and fonts. That purity is what
makes box-level memoization (M4) sound.

- **Paragraph builder / line breaker**: Knuth–Plass-style dynamic
  programming over a prepared horizontal list. `tex-typeset` exposes the pure
  decision pass (`line_break`) separately from post-line-break surgery
  (`post_line_break`). The breaker owns the three TeX passes: pretolerance
  without hyphenation, tolerance with a caller-supplied hyphenation hook, and
  emergency stretch. The execution-side hook follows TeX82's font gate and
  does not hyphenate a word when that font's `\hyphenchar` is outside the
  byte range. Legal breakpoints, demerits, fitness classes,
  `\looseness`, and line-penalty parameters are copied into plain structs at
  entry; the kernel never touches `Env`, `World`, or `&mut Universe`
  mid-algorithm. The decision pass scans the prepared list once in source
  order and presents legal breakpoints directly to its active frontier; it
  does not materialize paragraph-wide prefix-width or breakpoint tables.
  Each live route carries its cumulative starting widths, while completed
  history is reduced to compact passive break decisions and backpointers.
  Breakpoint-local width adjustments account for glue breaks and
  discretionary pre/replace widths without ad hoc line slicing. The
  immutable line-breaking snapshot also carries the
  `\leftskip` and `\rightskip` specs so their complete natural, stretch, and
  shrink widths participate in TeX82's background width for every candidate
  line before post-line-break surgery materializes the named glue nodes.
  Within each line-number and fitness class it retains
  TeX82's minimum-demerits route, including TeX's later-route replacement
  when two routes have equal demerits. Discretionary nodes carry their source
  kind, letting the pure breaker apply `\hyphenpenalty`, `\exhyphenpenalty`,
  consecutive hyphen demerits, and final-hyphen demerits without consulting
  state. Terminal-break final-hyphen costs are included before the breaker
  retains the minimum route for a line/fitness class, matching TeX82's
  candidate ordering rather than adjusting an already-pruned winner.
  The execution integration first calls the pure pretolerance entry point. It
  materializes a hyphenated alternate hlist through `Universe` only when that
  pass fails, then calls the pure tolerance/emergency entry point. Thus a
  successful first pass neither scans words nor freezes discretionary child
  lists. Pre-hyphenation follows TeX82 sections 894--899: candidates begin
  only after glue, skip permitted implicit kern and whatsit nodes, retain the
  active language/minima context, collect at most 63 same-font letters, and
  accept only TeX's permitted terminating nodes. As in TeX82, a word with no
  legal hyphenation point keeps its existing character, ligature, and kern
  nodes byte-for-byte instead of being reconstituted.
  The search result is a break plan independent of paragraph ownership.
  Execution consumes the popped paragraph list while lowering inline math;
  paragraphs without math return that allocation unchanged instead of cloning
  every node. It moves whichever owned list won (original or hyphenated) into
  a resumable post-line-break materializer. Execution directly measures one
  decoded line before requesting the next. After any overfull marker is
  appended, the state boundary validates handles while computing semantic
  identity in the same traversal, encodes and freezes the final children once,
  then returns the emptied node
  vector so its allocation is reused across the paragraph; migrating material
  is extracted in place without replacing that buffer. The materializer moves
  retained nodes rather than cloning the paragraph at either boundary.
  Post-line-break produces
  line node vectors with named
  `\leftskip`/`\rightskip` glue, per-line width/indent dimensions selected
  from `\parshape` first and otherwise TeX's `\hangindent`/`\hangafter`
  rules, and interline penalty decisions. The current `\parshape` payload is
  owned by `Universe` and referenced through an ordinary barriered token-list
  parameter, so local/global assignment, group restoration, checkpoints,
  semantic hashing, and format serialization all share the same state path;
  mode-list snapshots only copy its decoded value into immutable line-breaking
  parameters. Forced breakpoint penalties are not
  passable. The stomach remains responsible for extracting top-level
  mark/insert/adjust material from each line, freezing the retained line
  vectors, hpacking to the selected line width, applying the selected indent
  as the hbox shift, and appending hboxes, migrated contribution material, and
  after-line penalties to the enclosing vertical list in TeX order. Remaining
  pdfTeX corpus parity details are
  tracked as follow-up work rather than weakening this purity boundary.
- **Math list conversion**: `tex-typeset::math` owns the pure Appendix G
  kernel for the core noad-to-hlist pass. It consumes frozen mlist node lists,
  a starting style, a penalty flag, a plain `MathParams` snapshot, and
  read-only font/list/glue access; it returns an owned immutable hlist tree
  rather than freezing nodes itself, so the kernel has no `&mut Universe`
  surface. The implementation replays TeX.web's two passes: pass one resolves
  styles, math choices, mu glue/kerns, noad classes, nuclei, and scripts while
  tracking top-level dimensions; pass two inserts the 8x8 inter-class spacing
  table and, when the stomach is converting unrestricted paragraph math for
  line breaking, optional `\binoppenalty`/`\relpenalty` nodes. Restricted
  `\hbox` math uses the same inline style conversion without those line-break
  penalties. Converted inter-noad glue preserves diagnostic provenance for
  `\thinmuskip`, `\medmuskip`, and `\thickmuskip`, while explicit `\mskip`
  remains distinct; lowering and shipout treat those variants as ordinary glue
  for DVI-visible behavior. Symbol, extension, delimiter, and math penalty
  parameters are copied before entry, and the style helpers carry cramped
  propagation for recursive sublists. The pure kernel includes Appendix G
  compound builders for generalized fractions, radicals, big operators with
  displayed limits, variable delimiters and extensible recipes, math accents,
  over/under lines, and vcenter boxes under the same owned-output contract.
  Operator-axis centering follows TeX82's `make_op` character branch: a
  single math-character nucleus is centered on the math axis, while a compound
  nucleus such as `\mathop{\rm lim}` retains the baseline of its clean box.
  Limit switches apply equally to explicit `\mathop` noads and class-1
  `\mathchardef` operators. Displayed limits re-clean a shifted character
  operator before placing it in the limits vlist, but retain an already-clean
  compound operator box directly instead of introducing another wrapper.
  Appendix G `sub_box` nuclei retain their already-packed box node directly in
  the converted hlist, including an explicit `\raise`/`\lower` shift; they are
  not wrapped in a second hbox. `clean_box` still repacks fields where TeX
  requires a clean subsidiary box, such as scripts and compound noad
  construction.
- **Alignment (`\halign`/`\valign`)**: the one kernel that is *not* pure —
  template expansion interleaves with the gullet by design. It is
  structured as a stomach sub-mode (it re-enters main control per cell),
  not as a kernel function, and is therefore excluded from kernel-level
  memoization (page-level still covers it). `tex-exec` parses alignment
  preambles into snapshot-covered `AlignState` on the mode-list level:
  frozen u/v template token lists, frozen tabskip boundary glue ids, an
  end-template sentinel token, and optional `&&` repeat metadata. The
  preamble opener follows TeX's `scan_left_brace`, skipping expanded `\relax`
  commands before requiring the opening group; this is observable in recovery
  cases such as `\halign\relax{...}`.
  The
  repeat metadata maps both templates and their following tabskip boundaries;
  columns extended past the declared preamble therefore reuse TeX's periodic
  boundary glue instead of the final declared `\tabskip` value.
  The
  stomach alignment sub-mode now runs the row/cell loop, replays u/v
  templates through ordinary main control, recognizes unshielded `&`,
  `\span`, and `\cr` by meaning using an `AlignState` brace counter, buffers
  `\noalign{...}` material as ordinary internal-vertical nodes interleaved
  with the unset rows, and packages cells/rows as unset node records. An
  alignment-cell paragraph token at a negative brace level follows TeX.web's
  `hmode+par_end`/`off_save` recovery: the traced token is backed up behind an
  inserted provenance-bearing right brace. Each cell records the entry
  execution-group depth so ordinary u-template groups close normally, while
  a right brace at the alignment group takes `handle_right_brace`'s
  missing-`\cr` path. If that delimiter still arrives at a negative brace
  level, `align_error` inserts the matching left brace before ordinary
  v-template interception closes the cell and row. Outer
  macros are likewise stopped before expansion while an alignment cell is
  active, keeping the recovery input frames and origins checkpointable. An
  ordinary vertical `\halign` inherits the enclosing list's `prev_depth`,
  just as TeX.web's `push_nest` preserves `aux`, so the first row receives
  the same baseline glue as any other appended box. At
  `fin_align`, `tex-exec::align::widths` runs TeX.web's span-chain width
  resolution over those frozen rows, including tabskip-width subtraction and
  last-spanned-column excess placement, converts every reachable unset row/cell
  to ordinary hlist/vlist nodes, and emits row interline glue before the final
  raw splice for `\halign` so `\noalign` material resets row-to-row baseline
  insertion in TeX order. `\valign` uses the transposed path: cells execute in
  internal vertical mode with paragraph material finished before cell
  packaging, tabskip boundaries remain vertical glue inside each source-row
  vbox, and no row-to-row baseline glue is inserted between the side-by-side
  vboxes. A `\halign` encountered as the first display-math item takes the
  display-alignment branch: the alignment body is finished as vertical material
  between display penalties/skips, the closing `$$` is checked in the display
  path, and no `\eqno` material is accepted alongside it. Span-time template
  expansion remains the explicit architecture-§7 exception inside
  `tex-exec::align`; downstream page building, diagnostics, and shipout operate
  only on set boxes. Mid-alignment `AlignState` and unset rows/cells remain
  rollback-covered implementation data, but alignment execution emits no
  engine checkpoint. An edit inside an alignment resumes from the preceding
  named paragraph, shipout, or job-start boundary and replays the alignment
  normally.
- **Vertical packing, `\vsplit`, marks**: operate on survivor-arena lists
  (they are reachable from box registers by definition); mark extraction
  reads are recorded like any state read. `\vsplit` pins and reads the source
  vbox children directly, materializes only the top-level sequence it must
  partition, chooses its split with the shared pure `tex-typeset::vert_break`,
  writes only the split mark slots, prunes the
  survivor remainder with `\splittopskip`, and replaces or clears the source
  register through the same-level `Universe` box facade.
- **Status — implemented packing foundation**: `tex-typeset` currently provides pure
  `hpack`, `vpack`, `vtop`, `vert_break`, and TeX.web §108 badness over frozen node lists.
  The crate reads `Universe` immutably, including frozen nodes, glue specs,
  and loaded font character metrics, copies packing parameters into plain
  structs at entry, and returns box payloads, diagnostics, and the plain
  glue-setting badness without writing state. Stomach-side box-building
  primitives live in `tex-exec`; execution records the latest packing badness
  through `Universe` for the read-only `\badness` internal integer. When hpack
  diagnostics require TeX's overfull marker, `tex-exec` materializes the
  synthetic rule while freezing the final child list. Insufficient normal
  shrink is overfull whenever its residual exceeds `\hfuzz`, independently
  of the cubic badness value; diagnostics report that post-shrink residual.
  Infinite-order stretch
  or shrink sets glue with zero badness and never emits finite-order packing
  diagnostics, as in TeX82. Packed boxes retain the
  glue-set ratio as a reduced exact fraction, so cumulative TeX.web glue
  rounding is byte-stable without floating-point state or a lossy decimal
  approximation. The packing crate
  remains pure and has no `World` or `&mut Universe` surface.

## 8. Page builder and output routine

Responsibility: accumulate the main vertical list, fire `\output`, commit.

- The page builder is an incremental observer of main-vertical-list appends.
  The base vertical mode list keeps mode-local fields such as `\prevdepth`
  and `\prevgraf`; durable page state lives in `Universe`: the recent
  contribution list, current page nodes, `\pagegoal`/`\pagetotal` and the
  other `page_so_far` dimensions, `\insertpenalties`/`\deadcycles`, page
  contents state, page-level `\lastskip`/`\lastpenalty`/`\lastkern` mirrors,
  least-cost/best-break records, and the pending fire-up trigger. These fields
  are copied into snapshots and included in convergence hashes.
- **Status:** `tex-exec` currently ports the TeX.web accounting pass: discardables before
  the first box are pruned when the builder catches up; the first box freezes
  page specs from `\vsize`/`\maxdepth` and inserts adjusted `\topskip`; box,
  rule, glue, kern, and penalty contributions update page totals and legal
  breakpoint costs using TeX's `badness`/`awful_bad` comparison order. The
  insertion splitter and `\vsplit` share the pure `vert_break` kernel for
  least-cost vertical break selection. A forced or awful break records a
  pending fire-up boundary.
- Fire-up splits the current page at the recorded best break, rewrites the
  chosen break penalty to `10000`, stores the original penalty in
  `\outputpenalty`, updates `\topmark` from the old `\botmark`, scans the
  selected top-level page material for TeX82 mark nodes to set
  `\firstmark`/`\botmark`, distributes insertions to their class boxes, and
  vpackages the remaining page material into global `\box255` at the recorded
  best size using the captured `\maxdepth`. This follows TeX.web's `fire_up`
  ownership transfer: the packed page may combine fresh epoch lists with
  survivor-owned descendants introduced by copied boxes, so the aggregate box
  write promotes the mixed graph into one canonical survivor root before the
  epoch can roll back. Page-builder insert state is an
  ordered per-class record list in `Universe` page state: first insertion of a
  class applies the `\skip<n>` correction once, `\count<n>` scales natural
  insertion size in TeX.web order, `\dimen<n>` caps class material, and split
  remainders are held over with `\insertpenalties` reporting accumulated split
  penalties in mainline but held-over insertion count while `\output` runs.
  `\holdinginserts` keeps insert nodes in `\box255` instead of distributing
  them.
- **`\output` is a recursion**: an empty `\output` token list executes the
  default `\shipout\box255` path directly; otherwise the output token list
  replays as an input frame inside an implicit group and one internal-vertical
  nest level. `\shipout` is an ordinary primitive delivered back to the
  stomach, so output-routine assignments obey normal grouping and only
  `\global` writes survive the implicit group. At output end, non-void
  `\box255` is the TeX error, material left on the output nest level is
  prepended to the contribution list, `\deadcycles` enforces
  `\maxdeadcycles`, and a successful shipout resets the counter. Nothing
  special architecturally — except that
  **`\shipout` is the commit barrier** (`core_state.md` §9): the shipped
  page freezes, serializes into the content-addressed artifact store,
  deferred `\write`s expand *now* against current state (with read-set
  tracking active), the effect-log prefix flushes exactly once through
  `World`, and old history drops.
- `\end` runs the TeX-style final cleanup loop: while the page/contribution
  lists or dead-cycle state are not quiescent, it appends an empty hbox,
  fill glue, and an eject penalty, then lets the ordinary page builder and
  output routine fire until shipout drains the job.
- Page artifacts are the currency between the engine and both the output
  drivers and the incremental engine: a page artifact = (serialized node
  tree, resources used (fonts/images by content hash), `\count0..\count9`,
  frozen job metadata needed by output containers, effect slice).
  The concrete artifact substrate lives in `tex-out`: a versioned,
  hand-written binary format over lowered, driver-facing page nodes, font
  resource identities, `\count0..\count9`, and the page effect slice. The
  format is stored by content hash through `World`; drivers receive artifact
  bytes/ids, not live node handles. The real artifact store publishes each
  complete object with a same-filesystem temporary-file rename, so readers
  never observe a partial object. It does not force each page to stable storage
  or claim recovery across a process or machine crash.
  A successful aggregate shipout also publishes a process-local commit receipt
  pairing that authoritative id with the exact immutable canonical bytes.
  Fresh in-process drivers use the receipt; replay and out-of-process drivers
  continue to resolve the id through the verified artifact store. The receipt
  is never visible before both artifact storage and effect commit succeed.
- `tex-content` owns the fixed 32-byte content identity shared by `tex-state`
  and `tex-out`. New identities use the portable block-wise version-2 scheme
  and include a domain tag, so identical input and artifact bytes do not alias.
  Artifact reads verify the requested identity before decoding; domain-aware
  version-1 and pre-v1 undomained hashes are accepted only through the explicit
  legacy-read policy.
- **Status:** the implemented stomach shipout path consumes the same box
  syntax as TeX's
  box primitives (`\shipout\hbox{...}`, `\shipout\boxN`, `\shipout\copyN`),
  traverses the box tree in node order, fires deferred stream whatsits, expands
  deferred-write token lists through the ordinary gullet, serializes the
  `tex-out` artifact, and commits it through `Universe::commit_shipout`, which
  stores the artifact bytes, flushes the committed effect prefix, releases
  shipout-local epoch nodes, and advances the internal state-hash baseline.
  The outer executor publishes `ShipoutComplete` only after any output routine
  or other recursive caller has unwound; nested shipout itself is not a yield
  point.
  Before opening that commit boundary, shipout applies TeX's `max_dimen`
  checks to box height, depth, height-plus-depth-plus-`\voffset`, and
  width-plus-`\hoffset`; a huge page is diagnosed and discarded without an
  artifact, effect commit, dead-cycle reset, or checkpoint.
  Deferred `\openout` and `\closeout` whatsits append the same World stream
  records as `\immediate` stream commands, while deferred `\write` appends the
  routed stream-write record after shipout-time expansion. The same lowering
  applies TeX82's default `.tex` extension to an extensionless `\openout`
  target when the whatsit executes. Once shipout commits that ordered stream
  prefix, the materialized bytes are immediately available through the normal
  World input path later in the same job; staged or rolled-back stream effects
  never become readable.
  That traversal carries TeX.web's leader context: deferred stream open, write, and
  close whatsits inside leader payload boxes are ignored, while specials still
  become anchored page effects that the DVI leader loop emits for each
  repeated payload. The executor records shipped artifact ids for the
  CLI/driver layer. Shipout also prepares the job magnification before artifact
  construction and reports any
  recoverable `prepare_mag` diagnostic through the execution diagnostic/log
  path; `tex-out` only sees the resulting effective magnification in detached
  job metadata. Source-level
  `\special{...}` is implemented as a stomach whatsit whose balanced text is
  expanded at scan time, matching TeX82's `scan_toks(false,true)` behavior;
  shipout lowers each special whatsit into a `PageEffect::Special` and a
  `WhatsitAnchor` at the traversal position so DVI `xxx` output remains
  ordered by the committed box tree. Discretionary, mark, insert, and adjust
  nodes are also lowered into detached artifact nodes when they occur in a
  shipped box; DVI currently treats them as non-emitting metadata, preserving
  their payloads for later page-builder/mark/insert semantics without reaching
  back into live state.

## 9. Fonts and metrics (`tex-arith`, `tex-fonts`, `tex-state`)

Responsibility: every question about glyphs, loaded once, answered from
immutable tables, with mutable font state kept behind the state timeline.

- `tex-arith` owns TeX fixed-point arithmetic shared across scanners, state,
  and font parsing: `Scaled`, physical-unit and `true`-dimension conversion,
  `xn_over_d`/`nx_plus_y`, widened saturating scaled accumulation,
  `FontSizeSpec`, and TFM fix_word/font-size scaling helpers. The saturating
  helpers clamp only at the representable scaled boundary; legal TeX
  dimensions retain TeX.web's integer operation order and exact results. The
  crate has no dependency on state, fonts, or I/O.
- Loading and immutable font-domain data live in `tex-fonts`: TFM for classic
  compatibility; OpenType/TrueType via a
  vendored shaper for the modern path. All file access through `World`
  (fonts are inputs; cross-run memo sharing needs them pinned).
  The TFM parser is a TeX82-compatible validation boundary: its size words use
  `read_sixteen`'s 15-bit domain, section totals must equal `lf`, and complete
  trailing words after `lf` are ignored. Declared `bc..ec` membership is kept
  distinct from `char_exists` (a nonzero width index). Raw width-zero
  `char_info` tags are therefore structurally validated; next-larger links use
  range and cycle checks without requiring the target to exist, while
  lig/kern match and replacement operands and extensible recipe pieces apply
  TeX's stronger existence check (except the declared boundary character
  match). Rust additionally bounds every table index, restart, skip, and
  traversal before publishing the immutable metrics. The detached lig/kern
  program has one shared runtime capacity of 65,536 instructions: its final
  addressable index is `u16::MAX`, and an instruction at that index must stop.
  TFM parsing and format restore enforce the same bound, while the runtime
  iterator uses a checked cursor transition even for hand-built metrics.
- `tex-exec` applies TeX82's TFM filename rule (`.tfm` by default), then asks
  the driver hook to resolve the path through the narrow `InputReadState`
  capability. The CLI probes the principal input directory followed by the
  ordered nonempty directories in `TEXFONTS` for area-less names; an explicit
  font area is used as written, matching `read_font_info`'s `aire` branch.
  The successful read is the ordinary content-addressed `World` input record,
  so font search adds no engine-owned or non-checkpointable state. As in
  TeX82's separate `tfm_file` handle, that auxiliary record does not replace
  or renumber the active text source carried by the input stack.
- A loaded font is an immutable object; `FontId` is the state-layer handle
  (`core_state.md` §10.3) minted at load time; `\font` assignment is an
  ordinary barriered `Env` write. Per-font mutable parameters
  (`\fontdimen`) live in `Env`-side banks, *not* in the font object —
  loaded fonts stay immutable and shareable across snapshots and threads.
- `tex-state` stores loaded `tex-fonts` records in its `FontStore`, but only
  owns stateful concerns: `FontId` minting/liveness, rollback, current-font
  selectors, hyphenchar/skewchar, `\fontdimen` banks, and read-only `Universe`
  facades. `tex-fonts` must not depend on `tex-state`.
- Loaded fonts carry backend-neutral immutable metrics owned by `tex-fonts`:
  character
  width/height/depth/italic, TFM-style ligature/kern pair answers including
  boundary programs and ligature retention/pass-over bits, and extensible
  recipes. Kernels consume these through read-only `Universe` methods keyed by
  `FontId`; they do not inspect TFM parser tables or store internals.
- Classic byte-character metrics also derive a dense 256-entry width array at
  load time. `tex-typeset` consumes it through the read-only state facade while
  scanning opaque contiguous same-font compact-node runs. The parallel full
  character table supplies height/depth without repeated font validation.
  Both are immutable projections of the same font input, not timeline caches;
  Unicode/non-TFM glyphs and interrupted runs stay on the scalar accessor path.
- Font parameters are intentionally separate from those immutable metrics.
  `Universe::font_parameter(font, n)` reads the Env-side `\fontdimen` bank, so
  runtime writes are visible to scanners and kernels; the original TFM
  parameter values only seed those banks at load time. TFM parsing and the
  backend-neutral loaded-font boundary both enforce TeX82's guaranteed
  `\fontdimen1` through `\fontdimen7`, padding absent values with zero before
  the snapshot-covered Env bank is initialized. The journal cell codec splits
  its 30-bit index evenly between a 15-bit dense `FontId` and a 15-bit
  zero-based parameter slot: font ids `0..=32767` and fontdimens `1..=32768`
  are injective, with the final pair mapping to index `(1 << 30) - 1`.
  Runtime loading and assignment preflight reject either field beyond that
  domain before publishing a font or changing parameter-count state; invalid
  reads use TeX's zero-valued dummy font-info behavior.
- Later OpenType support should lower backend data behind the same boundary:
  glyph metrics can populate the immutable metric record, while complex
  shaping can replace the TFM pair-program implementation without exposing
  GSUB/GPOS details to paragraph or math kernels.

## 10. Output drivers (`tex-out`)

Responsibility: page artifacts → bytes on disk. Strictly downstream.

- Drivers consume committed page artifacts only — they can run
  out-of-process, in parallel with typesetting of later pages, or not at
  all (editor preview may rasterize page artifacts directly).
- `tex-out` owns the page artifact model and version-11 binary reader/writer.
  Exact glue-set numerator and denominator fields cross this commit boundary
  and participate in deterministic semantic hashing. `GlueSetRatio` performs
  checked canonical reconstruction at every serde boundary, and the artifact
  reader applies the same fallible constructor explicitly: nonpositive
  denominators and unrepresentable magnitudes are rejected, while reducible
  ratios and zero are normalized before they enter packing or driver code.
  The crate has no
  dependency on `tex-state` or `Universe`; shipout code lowers live state into
  artifact bytes before asking `World` to store them.
- Box shifts retain TeX.web's `shift_amount` representation across live state,
  format images, committed artifacts, and drivers: positive is down in an
  hlist and right in a vlist. Format-image version 5 carries version-2 content
  identities; version 4 established the current shift representation. Artifact
  version 11 and format version 5 reject older ambiguous encodings or identity
  schemes rather than guessing context or silently changing semantic hashes.
- The artifact record captures the effective job magnification, banner,
  `\hoffset`, and `\voffset` at shipout, so DVI generation does not reach back
  into live state. The offsets are read through `Universe` at the commit
  boundary and remain part of the detached, checkpoint-safe page artifact.
- PDF driver owns the PDF object model; `\pdfliteral`-class primitives
  produce *effect-log entries* engine-side that the driver interprets —
  the engine never constructs PDF syntax.
- DVI driver is the conformance driver: byte-comparable against Knuth's
  `tex` for the parity corpus. **Status:** the implemented DVI layer writes
  the file container structure (`pre`, page `bop`/`eop`, first-use `fnt_def`, `post`,
  `post_post`, and 223 padding) from committed artifacts, and traverses the
  committed box tree with TeX.web-style `hlist_out`/`vlist_out`, `movement()`
  w/x/y/z optimization, font switches, rules, glyphs, DVI specials, and leader
  placement.
  Cumulative glue setting preserves TeX82's operation order: the exact packed
  ratio is converted to binary `real`, multiplied by the running glue total,
  clamped, and rounded before each movement delta is emitted.
  DVI font numbers are the driver-visible TeX font numbers derived from
  `FontId` load order, not artifact-local dense renumbering, so INITEX parity
  cases that load several sizes/families preserve reference font selection
  bytes.
  Its sink-oriented writer decodes, validates, emits, and drops one page
  artifact at a time; only global postamble data, indexed font definitions,
  movement state, and the current page buffer remain resident.
  The `umber run file.tex --dvi out.dvi` CLI path is a thin downstream
  composition over shipout commit receipts: it parses their canonical bytes
  as `tex-out` page artifacts and invokes the DVI writer without a second
  store read, content hash, or access to live `Universe` state. The public
  ID-only composition retains the read-and-verify path for durable replay.
  The DVI parity
  harness runs the reference engine live and byte-compares outputs after the
  single sanctioned normalization: replacing the existing preamble comment
  payload bytes in both files so the Umber/reference banner text differs
  without masking any other byte, length, or pointer discrepancy.
- Because drivers see only committed artifacts, rollback never reaches
  them; there is nothing to undo downstream of the commit barrier.

## 11. Incremental engine (`tex-incr`)

Responsibility: turn the state layer's bookkeeping into speed. This crate
is the *driver* for editor sessions and warm rebuilds; batch mode is the
degenerate case (run once, commit every page, never look back).

- **Convergence-based reuse** (`core_state.md` §9): on edit, roll back to
  the latest retained named boundary before the edit point, re-execute, and
  compare `state_hash` only at the same named-boundary schedule; on match,
  splice the previous run's suffix of page artifacts and stop.
  `incremental_v1.md` fixes the exact boundary schedule, editor-revision
  mapping, retained logical-shipout policy, pruning order, and suffix-splice
  contract.
  The hash is a semantic checkpoint hash, not a store-layout checksum:
  content handles are followed to token/glue/node/macro contents, control
  sequences are keyed by name, and checkpoint hashes are combined from the
  previous checkpoint plus the current semantic slice. Store cells retain a
  derived semantic fingerprint at the latest boundary, so the next slice
  fingerprints final-live content once and compares it with that baseline
  instead of re-walking both old and final content. Rollback clears this cache
  and reconstructs missing baselines from journal old words; it is not part of
  the snapshot or semantic state tuple.
  Checkpoint-hash schema version 5 extends that rule to non-journal state with
  domain-separated component projections keyed by immutable roots or cheap
  semantic cursors. Stable code tables, hyphenation, stream buffers, input
  roots, page subroots, and mode roots reuse their canonical fingerprints.
  Input checkpoint semantics are published as one immutable `InputSemanticRoot`
  rather than a mirrored field cursor.
  A changed input root is projected once and compared by canonical fragment,
  allowing semantically equal rebuilt roots to retarget the cursor without
  adding a false schedule-relative state transition.
  Current-page nodes are stored as a position-canonical binary forest of
  immutable 64-node leaves. Leaves and branches lazily memoize their canonical
  projection when a checkpoint first reaches them; ordinary append does no
  hashing, and forks reuse memoized fragments through the shared tree.
  Leaf projection visits its bounded outer nodes, while frozen child lists
  contribute their versioned canonical `NodeSemanticId` without reopening
  compact node storage.
  Projection caches remain private derived accelerators; a shared private
  cache-entry abstraction keeps reuse keys separate from canonical fragments,
  and pointer identity is never part of a hash value. Page-tree memoization
  lives only as long as its immutable tree and is safe to retain across
  rollback; the current sub-64-node tail uses the ordinary discardable root
  cache.
  Every published checkpoint is restartable. If an edit falls inside an
  alignment, box, scanner, inline formula, or output routine, the session
  selects the preceding published boundary and replays the whole construct.
  When convergence adopts an old suffix, every adopted checkpoint is eagerly
  rehomed onto the new accepted root-buffer revision through an engine-owned
  validated transformation. Accepted history therefore has no revision-map
  chains and remains directly restartable by the next edit. After a splice the
  session exposes detached accepted output and checkpoint history, not a
  fictional job-end live executor state.
- **Memoization** begins at pure kernel boundaries, especially paragraph line
  breaking keyed by immutable hlist content plus captured parameters. Generic
  interpreter read-set memoization remains deferred until a measured consumer
  justifies its cross-cutting instrumentation.
- **Speculative page execution** is not part of incremental v1. It remains a
  future research item to be reconsidered only after boundary-scoped
  convergence and pure-kernel memoization demonstrate measurable value.
- The engine exposes one API to the CLI/editor: `advance(to: EditPoint) ->
  impl Iterator<Item = PageArtifact>`. Everything above is strategy.

## 12. JIT (`tex-jit`, M5)

Responsibility: compile hot token programs (macro bodies, expl3 code)
to native code that plays by the rules.

- Consumes the sealed `layout` contract: stable cell addresses, epoch
  stamps as inline-cache guards, the barrier as a specified instruction
  sequence (`core_state.md` §10.7).
- Tiering: interpreter → baseline (straight-line meaning-dispatch
  elimination for stable macros) → optimizing (speculate on catcode
  generations and parameter values, deopt on epoch mismatch).
- Correctness is differential, not axiomatic: byte-identical journals and
  state hashes vs. the interpreter on fuzzed programs. A JIT that skips a
  barrier fails replay identity immediately.

---

## 13. Cross-cutting invariants (the contract between subsystems)

1. **No hidden checkpoint state.** If a subsystem holds anything mutable
   across an engine checkpoint boundary, it is either (a) inside `Universe`,
   (b) rooted in the checkpoint tuple (input stack, mode nest, page-builder
   scalars, condition stack), or (c) a pure cache validated by
   epochs/generations. Ordinary synchronous locals may exist only while a
   boundary is forbidden.
2. **Demand-driven pipeline.** Downstream pulls from upstream; nothing
   buffers tokens across a state write except under a generation guard.
3. **Kernels are pure.** Typesetting algorithms read parameters at entry
   and never mutate. Alignment is the documented exception and is
   structured as stomach recursion instead.
4. **Effects only via `World`; commits only via `Universe`.** Any new
   primitive with an observable side effect gets an effect-log entry kind,
   not an I/O call. Uncommitted records are rollback state; committed
   prefixes are materialized through `World` and then discarded from the
   in-memory log, but the public commit boundary is `Universe`: it flushes the
   `World` prefix and updates aggregate checkpoint/hash bookkeeping as one
   operation. Downstream code may receive narrow world view/I/O capabilities;
   it must not receive timeline-control authority such as raw effect commits,
   snapshots, rollbacks, or hash cursors.
5. **One interpreter loop.** Expansion and execution share the
   `get_x_token` core; there are not two token-reading engines with subtly
   different semantics (a classic source of TeX-clone divergence).
6. **Conformance is layered**: lexer vs. pdfTeX token dumps → gullet vs.
   expansion traces → stomach vs. `\showlists` → pages vs. DVI bytes.
   Each pipeline stage gets a differential harness against the reference
   *before* the next stage builds on it (mirrors `core_state.md` §11.5).
7. **Live identity is generation tagged, serialized identity is semantic.**
   Every rollback-truncated store uses the state layer's common
   `(namespace, generation, slot)` allocator and O(1) tag validation. A
   rollback generation never rewinds or wraps, and forked timelines share only
   inherited handles. Durable formats and committed artifacts never serialize
   these runtime capabilities: they encode validated DTO-local references or
   content identities and mint fresh handles only while reconstructing the
   aggregate live state.

## 14. Build order

The state layer's milestones M1–M5 (`core_state.md` §13) interleave with
pipeline milestones:

| Phase | Pipeline deliverable | Rides on state milestone |
|---|---|---|
| P1 | Input layer + semantic lexer; token dump parity | M1 (interner, Env) |
| P2 | Gullet: macros, conditionals, expandables; expansion-trace parity | M2 (token store) |
| P3 | Stomach: assignments, grouping, number scanning; `\showlists` parity on non-typeset ops | M1/M2 |
| P4 | Boxes, paragraph builder, line breaker, fonts (TFM); simple pages via DVI, byte parity | M2/M3 |
| P5 | Page builder, output routine, marks/inserts; math; full DVI corpus parity | M3 |
| P6 | SIMD lexer, incremental engine v1 (convergence splicing) | M4 |
| P7 | Pure-kernel memoization and JIT baseline | M4/M5 |

The guiding rule, as in the state plan: every guard and every piece of
bookkeeping a later phase needs (generations, epochs, read-sets, artifact
hashes) already exists — earlier phases must not invent private shortcuts
that a later phase has to unwind.
