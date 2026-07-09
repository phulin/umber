# Engine Architecture — Core Subsystems

Status: draft companion to [core_state.md](core_state.md)
Scope: everything *around* the state layer — the processing pipeline
(input → lexing → expansion → execution → typesetting → page building →
shipout), the service layers (fonts, output drivers), and the advanced
consumers (incremental engine, JIT). The state layer itself (`tex-state`)
is specified in `core_state.md` and treated here as a given.

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
> state) — which is what makes snapshots, memoization, speculation, and the
> JIT *queries and consumers* rather than invasive rewrites.

```text
                              ┌────────────────────────────────────────────────┐
                              │              INCREMENTAL ENGINE                │
                              │  convergence · memoization · speculation       │
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
  stage reads meanings/codes from `Universe` and writes through the
  barrier. The pipeline stages hold **no hidden semantic state of their own**
  beyond what the snapshot tuple captures (input stack summary, mode nest,
  stream buffers — see `core_state.md` §9). In the current recursive
  interpreter, snapshots are resume-valid only at explicit engine quiescence
  boundaries. Checkpoints taken while a nested stomach scanner is active are
  hash-only: they advance convergence hashing, but their metadata points
  execution resume back to the previous resume-valid boundary until a future
  incremental executor defines serialized continuation points.

## 2. Crate map

```text
umber (CLI / driver)
 ├── tex-incr        incremental engine: convergence, memo store, speculation
 │    └── tex-engine       the interpreter proper
 │         ├── tex-expand       gullet: macros, conditionals, expandable prims
 │         ├── tex-exec        stomach: mode machine, unexpandable prims
 │         │                   snapshots execution parameters for pure kernels
 │         ├── tex-typeset    par builder, line break, math, alignment, page
 │         │                   consumes immutable state views and plain params
 │         ├── tex-lex        eyes+mouth: decoding, catcodes, input stack
 │         └── tex-fonts      font loading, metrics, shaping
 ├── tex-out         drivers: PDF, DVI, (later) HTML — consume page artifacts
 ├── tex-jit         privileged consumer of tex-state::layout (M5)
 └── tex-state       the state layer (core_state.md) — everybody's substrate
```

Dependency rules (enforced in CI like the effect lints):

- Only `tex-state` owns mutation and history. No other crate defines a
  store, a cache keyed by mutable state, or interior mutability.
- Only `tex-fonts` and `tex-out` may hold large immutable resources
  (font binaries, ICU tables); they are loaded through `World` so inputs
  stay content-addressed.
- `tex-jit` is the sole consumer of the sealed `layout` module.
- Nothing depends on `tex-incr`; it depends on everything. Incrementality
  is a *driver strategy*, not a property scattered through the pipeline.

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
- **Decoding**: UTF-8 native. Legacy 8-bit input is a per-source decoder
  selected up front, not a per-character branch in the lexer.
- **The input stack** is the one piece of pipeline-owned state the snapshot
  must summarize (`InputSummary`): a vector of frames, each either a
  *source frame* (source id + source byte offsets + line/col + current
  normalized line + in-line char/byte offsets + lexer state + pending
  synthetic tokens) or a *token-list frame* (`TokenListId` + index + replay
  kind: macro body, `\everypar`, mark, ...). Macro-body token-list frames
  additionally carry up to nine frozen argument `TokenListId`s; replaying a
  `Param(slot)` token pushes the corresponding argument list as a nested
  macro-argument frame while the replacement body id remains unchanged.
  Open conditionals are summarized as condition frames in the same vector,
  not as expansion-owned side state. A condition frame records whether it is
  a regular `\if...` or `\ifcase`, the current limb (`\if`, `\or`, or
  `\else`), whether the condition is still evaluating its operands, whether
  the current and any previous limb has been taken, the `\ifcase` `\or`
  count, and the nested conditional depth observed during skip/resume scanning.
  Source reopen identity is owned by the `World` input record in the outer
  snapshot: it pins file/editor content by content hash, reopens that exact
  source, then applies the lexer-owned source-frame summary.
  `last_source_frame` is also summarized so snapshots taken just after a
  source pops still have final source coordinates.
- Line-oriented details TeX cares about (`\endlinechar`, trailing-space
  trimming, `%` line ends) live here, driven by parameters read through the
  aggregate state API. The implemented `tex-lex` input layer exposes a local
  `InputSource` trait for memory buffers and files, normalizes each physical
  line by trimming trailing spaces and appending the current `\endlinechar`
  when valid, reports blank/all-space lines with a valid appended endline
  character as a structured paragraph boundary event for the semantic lexer
  to turn into `\par`, and owns an
  `InputStack` whose `InputSummary` records resume-complete lexer-owned
  source frame state and token-list replay progress. Token-list frames read
  frozen content only through `Universe::tokens`; durable source identity is
  the `World` input record captured in `Universe` snapshots, which pins file
  bytes by content hash so a driver can reopen the exact source and apply the
  lexer summary. `\endinput` is represented as a source-frame flag that lets
  the lexer finish the current normalized line and then pop that source
  without asking expansion to manage source internals.

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
- Control sequence names intern immediately to `Symbol`; the lexer emits
  `Token = Char(char, Catcode) | Cs(Symbol)` — one word, `Copy`.
- The lexer holds **no state outside the input stack frame** (its N/M/S
  state is part of the frame). Nothing here needs journaling.

## 5. Expansion engine (the gullet)

Responsibility: the token-level rewriting system — macros, conditionals,
`\expandafter`, `\csname`, `\the`, `\number`, e-TeX expandables.

- **Structure**: a `get_x_token` loop. Pull a token; look up its meaning
  word in `Env` (one load); if expandable, push its expansion as a
  token-list frame and continue; else deliver it downstream. Control-sequence
  tokens address their interned symbol directly; active character tokens
  address the same one-character symbol used by definition assignments.
- **Macro call**: match delimited/undelimited parameters against the
  incoming stream (argument scanning is the gullet's inner loop and the
  #1 profile target); arguments are built with token builders and frozen.
  Macro meanings decode through the state aggregate into flags plus frozen
  parameter-text and replacement-text token-list ids. The body replays as a
  frame with argument slots resolved by index — no token-list copying for
  the body, ever (bodies are shared `TokenListId`s).
- **Macro definition scanning**: `tex-expand` exposes the shared
  `scan_toks`-style scanner for `\def`/`\edef` syntax. It scans parameter
  text and a brace-balanced replacement body into frozen token lists,
  including TeX's ordered `#1`..`#9` parameter markers, trailing `#{`, and
  doubled `##` replacement-body escapes. The scanner may freeze content
  through `Universe`, but it does not assign meanings; the stomach/future
  assignment layer remains responsible for installing the returned
  `MacroMeaning`.
- **Numeric value scanning**: `tex-expand::scan_int`,
  `tex-expand::scan_dimen`, and `tex-expand::scan_glue` own the reusable
  expanded integer, dimension, glue, and muglue scanners for conditionals and
  the later stomach assignment layer. They pull
  through `get_x_token`; the integer scanner understands TeX integer
  constants and currently readable integer-like state (`\count`, chardef
  values, `\endlinechar`, and raw-sp `\dimen` coercion), while the dimension
  scanner parses decimal constants, physical units, `true` units, supported
  internal dimensions, `mu` dimensions for muglue callers, infinite `fil`
  orders for glue components, and opt-in integer-to-sp coercion. The glue
  scanner parses optional `plus`/`minus` components and interns immutable glue
  specs through `Universe`. These scanners report recoverable numeric diagnostics
  without performing assignments; execution-side assignment callers route those
  diagnostics through TeX's terminal/log diagnostic path before applying the
  recovered value. `true` physical units call the
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
  read `Env` and mint fresh frozen token lists.
- **Read-set recording** hooks live here and in the stomach: when the
  incremental engine asks for it, meaning lookups record `(cell, epoch)`
  pairs (`core_state.md` §9). Off by default, zero-cost when off (the
  recorder is a generic parameter of the loop, monomorphized away).
- The implemented `tex-expand` scaffold exposes that loop over
  `tex-lex::InputStack` through the shared `ExpansionState` capability, not
  broad `&mut Universe`. Production callers wrap the owning `Universe` in
  `ExpansionContext` before entering the gullet. That capability allows meaning
  reads, immutable token/glue/font/node/register/parameter reads, token-list
  freezing, glue interning, magnification preparation, lexer control-sequence
  interning, and `\csname`'s relaxed control-sequence interning. `ExpansionState`
  cannot construct input-read authority; the top-level expansion/dispatch path
  additionally carries `InputOpenState` only so `\input` can create an
  `InputOpenContext`. Scanner and helper recursion does not receive that
  authority directly. Instead recursive expanded-token reads go through the
  narrow `ExpandNext` capability; the top-level driver supplies a
  `DriverExpandNext` implementation that can re-enter dispatch with `\input`
  authority, while ordinary helper-only paths use a no-input implementation.
  Dimension, glue, condition-token, register-index, and `\the` operand scans
  therefore expose both no-input helper entry points and explicit
  expander/driver-aware entry points for production callers that already own
  input-read authority.
  File reads for `\input` live behind the separate `InputReadState`
  capability; driver hooks receive an `InputOpenContext`, not `ExpansionState`,
  so hooks can open input files without seeing meaning reads, Env/register
  writes, code-table writes, grouping, snapshot, font-assignment, or general
  World mutation APIs. Macro body replay uses
  the body `TokenListId` directly plus frozen argument ids on the replay
  frame; it does not allocate a substituted body list. Token-list replay is
  naturally read-only; source-frame replay may intern newly encountered
  control sequence names through the lexer/interner capability. `\csname` uses a dedicated
  expansion scan that stops on `\endcsname`, validates that expanded name
  material is character tokens, and interns/relaxes the resulting control
  sequence through the same aggregate boundary. Primitive installation and
  stomach assignment/test setup helpers still receive `&mut Universe`, but
  the production token-reading and scanner path is Rust-enforced against
  Env/register/code-table writes.
- Frame-control expandables are represented as input-frame rewrites:
  `\expandafter` saves one raw token, performs one expansion step on the
  following token, then pushes the saved token above the expansion result;
  `\noexpand` pushes a one-token replay frame that suppresses expansion for
  exactly the next `get_x_token` read. This keeps suppression frame-local and
  avoids mutating `Env`.
- Implemented conditional predicates evaluate in `tex-expand` and record their
  result by pushing/updating `tex-lex` condition frames. `\if` and `\ifcat`
  expand only to the two unexpandable comparison tokens; `\ifx` reads two raw
  tokens and compares macro meanings by flags plus hash-consed
  macro-definition ids, with non-macro control sequences falling back to
  meaning-word equality. `\ifnum`, `\ifdim`, `\ifodd`, and `\ifcase` reuse the
  shared integer/dimension scanners, including `\ifcase` `\or` limb selection.
  Mode predicates read only a driver-supplied query trait; box predicates read
  only the `Universe` box-register facade; `\ifeof` reads the `World` input
  stream table through the expansion hook's `Universe` access. False
  conditional limbs and already-taken `\ifcase` limbs are
  skipped by reading raw tokens from `tex-lex` under the active catcode table,
  while `\else`, `\or`, and `\fi` update the input-stack condition frame and
  report extra-control, incomplete-conditional, and skipped-outer-token
  diagnostics. A delimiter expanded while the current condition is still
  evaluating its operands is handled like TeX.web's `insert_relax`: the
  delimiter is pushed back and a relaxed token is inserted ahead of it.
- Value-rendering expandables (`\string`, `\number`, `\romannumeral`,
  `\meaning`, and the currently supported `\the` classes) mint their visible
  output through the explicit token-list freezing capability. `\the` covers
  integer, dimension, glue, muglue, and token registers; register aliases;
  integer, dimension, glue, muglue, and token parameters represented in `Env`;
  and code-table values. Named math glue parameters such as `\thinmuskip`,
  `\medmuskip`, and `\thickmuskip` are exposed as muglue assignment targets
  while their values remain in the glue-parameter bank at TeX's parameter
  indices. Font dimensions, box dimensions, page state, and
  time/job parameters not yet backed by `Env` remain documented TODOs until
  those classes are semantically available.
- Input/job expandables use explicit driver hooks: `tex-expand` scans the
  `\input` file name and asks the caller for a new `InputSource`, while
  `\jobname` renders the caller-provided job name. This preserves the rule
  that file access belongs to `World`/the driver layer, not to the gullet.
  `\fontname` renders loaded font selector names. The mark-family expandables
  replay the frozen token lists stored in the Universe-owned page mark slots;
  empty slots replay the canonical empty token list.
- The stomach implements the macro-definition assignment surface used by the
  expansion conformance path: `\def`, `\edef`, `\gdef`, `\xdef`, `\let`,
  `\futurelet`, prefix accumulation (`\global`, `\long`, `\outer`,
  `\protected`), and `\globaldefs` override behavior. Definition targets use
  TeX's `get_r_token` rule: either a control sequence or an active character
  is accepted, with active characters stored under their one-character symbol.
  These commands scan through the shared gullet/token scanner where expansion
  is required and write meanings only through the barriered `Universe` facade. The
  `umber expand-dump` driver delegates those primitives to `tex-exec` before
  printing delivered tokens; its remaining local assignment handling is
  limited to dump-corpus scaffolding such as `\chardef` and `\catcode` until
  those stomach assignments land.
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
  mode stack (nest) tracks the list under construction per level; the
  current list is an *unfrozen node builder* (§7 of `core_state.md` —
  builder-then-freeze applies to node lists exactly as to token lists).
- **Assignments** are thin: decode operand, scan value (number/dimen/glue
  scanning lives here — it consumes expanded tokens), call `Env::set` /
  register setters. `\global` maps to the tagged journal write. Grouping
  primitives (`{`, `}`, `\begingroup`, `\endgroup`) map to journal
  group markers — the stomach contains **no save-stack logic**; it calls
  `universe.enter_group()` / `leave_group()` and `\aftergroup` tokens are
  the group marker's payload.
- The arithmetic-only substrate for dimension scanning lives in
  `tex-state::scaled`: TeX's `xn_over_d` conversion routine, decimal fraction
  rounding, the physical-unit conversion table, and the `max_dimen` range
  check with the canonical `Dimension too large` diagnostic. Token parsing,
  signs, `true` magnification, internal units, and assignment effects remain
  scanner/stomach responsibilities.
- **Box building**: `\hbox{...}` etc. scan a packing spec, enter the brace
  group as a normal journal-backed group, execute a nested
  restricted-horizontal or internal-vertical list builder, freeze the
  finished list into the epoch arena, then call the pure `tex-typeset`
  packing kernel while the box-local assignments are still visible. The
  group is left before the resulting box is stored or appended, so only
  global assignments survive outside the builder. When horizontal packing
  reports an overfull box and
  `\overfullrule` is positive, the execution hand-off appends TeX's
  running-height rule node to the packed child list before the box can be
  stored, appended, or shipped. Storing the resulting one-node list in a box
  register is the barriered promotion write. Pulling boxes back out through `\copy`,
  `\box`, unboxing, `\lastbox`, or box-dimension rewrites clones any
  survivor-backed node tree into the current epoch before it can be appended
  to an unfinished mode list or promoted again. Horizontal list construction
  buffers adjacent font-backed characters in the stomach until a boundary
  command, then reconstitutes them through the loaded font's TFM ligature/kern
  program, updates the mode-local `\spacefactor`, and appends explicit
  h-mode nodes for spaces, kerns, skips, finite-fill glue, penalties, rules,
  discretionaries, accents, and italic corrections. Paragraph breaking is a
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
  The mode-list summary carries the pending incomplete fraction so snapshots
  preserve TeX's `\over`/`\atop`/`\above` state.
  `\mathcode"8000` redispatches through the current active-character meaning
  at use time, and family font selectors live in the barriered Env font state.
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
- **Paragraph and page hand-off**: paragraph start/end is stomach-owned.
  `\indent`, `\noindent`, implicit start from vertical-mode character
  material, `\parskip`, and `\everypar` replay are handled before entering
  unrestricted horizontal mode. When horizontal material ends (`\par` or
  `\endgraf`), the stomach performs TeX's final paragraph-list preparation
  (trailing-glue removal and `\penalty10000` plus `\parfillskip`), expands
  finished inline math lists into hlist nodes bracketed by `\mathsurround`
  `MathOn`/`MathOff` markers, snapshots paragraph-shape and line-breaking
  parameters, calls the pure line breaker over the prepared hlist, runs
  separate post-line-break surgery, freezes each resulting line list, hpack's
  it to the captured line width, and appends the hboxes through the shared
  vertical append routine. Fresh engine state
  installs the plain-format paragraph/layout defaults that affect this hand-off
  (`\pretolerance=100`, `\tolerance=200`, `\baselineskip=12pt`,
  `\parfillskip=0pt plus 1fil`, `\overfullrule=5pt`, and `\maxdepth=4pt`) so
  parity fixtures only restate them when a case intentionally overrides the
  format baseline. The page builder (§8) observes appends to the main vertical
  list.
- The stomach is the *only* pipeline stage holding `&mut Universe`, and it
  holds it as a plain argument — re-entrancy (e.g. `\output` routines,
  `\vsplit`-triggered mark extraction) is recursion in Rust, with the mode
  nest making it explicit and snapshot-summarizable. Recursive stomach
  scanners whose continuation phase is not serialized must bracket their work
  as hash-only checkpoint scopes. The current implementation does this for
  box-group scanning, alignment row/cell execution, `\noalign` groups, and
  alignment template replay; snapshots inside those scopes remain rollback
  and hash checkpoints, but drivers must resume from the previous
  resume-valid boundary and replay forward.
- The implemented `tex-exec` scaffold owns that explicit mode nest now. Its
  summary is a vector of mode levels, each carrying one of TeX's six modes
  (vertical/internal vertical, horizontal/restricted horizontal, math/display
  math) plus the node list under construction. Main control pulls through
  `tex-expand`'s `get_x_token` loop, and the box-group scanner re-enters the
  same dispatch path for nested stomach work. The gullet's mode predicates
  are backed by the current nest level through `ExpansionHooks`, collapsing
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
  emergency stretch. Legal breakpoints, demerits, fitness classes,
  `\looseness`, and line-penalty parameters are copied into plain structs at
  entry; the kernel never touches `Env`, `World`, or `&mut Universe`
  mid-algorithm. The decision pass keeps prefix width totals and
  breakpoint-local width adjustments so glue break widths and discretionary
  pre/replace widths are accounted for at the breakpoint rather than by
  ad hoc line slicing. Discretionary nodes carry their source kind, letting
  the pure breaker apply `\hyphenpenalty`, `\exhyphenpenalty`, consecutive
  hyphen demerits, and final-hyphen demerits without consulting state.
  The current `tex-exec` integration precomputes the hyphenated hlist because
  automatic hyphen insertion still freezes discretionary child lists through
  `Universe`; the breaker only sees the hook result as ordinary nodes.
  Post-line-break produces line node vectors with named
  `\leftskip`/`\rightskip` glue, per-line width/indent dimensions selected
  from `\parshape` first and otherwise TeX's `\hangindent`/`\hangafter`
  rules, and interline penalty decisions. Forced breakpoint penalties are not
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
- **Alignment (`\halign`/`\valign`)**: the one kernel that is *not* pure —
  template expansion interleaves with the gullet by design. It is
  structured as a stomach sub-mode (it re-enters main control per cell),
  not as a kernel function, and is therefore excluded from kernel-level
  memoization (page-level still covers it). `tex-exec` parses alignment
  preambles into snapshot-covered `AlignState` on the mode-list level:
  frozen u/v template token lists, frozen tabskip boundary glue ids, an
  end-template sentinel token, and optional `&&` repeat metadata. The
  stomach alignment sub-mode now runs the row/cell loop, replays u/v
  templates through ordinary main control, recognizes unshielded `&`,
  `\span`, and `\cr` by meaning using an `AlignState` brace counter, buffers
  `\noalign{...}` material as ordinary internal-vertical nodes interleaved
  with the unset rows, and packages cells/rows as unset node records. At
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
  only on set boxes. Mid-alignment `AlignState` and unset rows/cells are part
  of the mode-list summary and rollback-covered node data, but the current
  interpreter marks checkpoints taken inside alignment execution as hash-only.
  Incremental drivers should use those hashes for convergence, then roll back
  to the recorded resume-valid boundary before the alignment and replay
  forward until resumable continuations are designed explicitly.
- **Vertical packing, `\vsplit`, marks**: operate on survivor-arena lists
  (they are reachable from box registers by definition); mark extraction
  reads are recorded like any state read. `\vsplit` clones the source vbox
  children back to epoch storage, chooses its split with the shared pure
  `tex-typeset::vert_break`, writes only the split mark slots, prunes the
  survivor remainder with `\splittopskip`, and replaces or clears the source
  register through the same-level `Universe` box facade.
- **Implemented packing foundation**: `tex-typeset` currently provides pure
  `hpack`, `vpack`, `vtop`, `vert_break`, and TeX.web §108 badness over frozen node lists.
  The crate reads `Universe` immutably, including frozen nodes, glue specs,
  and loaded font character metrics, copies packing parameters into plain
  structs at entry, and returns box payloads, diagnostics, and the plain
  glue-setting badness without writing state. Stomach-side box-building
  primitives live in `tex-exec`; execution records the latest packing badness
  through `Universe` for the read-only `\badness` internal integer. When hpack
  diagnostics require TeX's overfull marker, `tex-exec` materializes the
  synthetic rule while freezing the final child list. The packing crate
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
- `tex-exec` currently ports the TeX.web accounting pass: discardables before
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
  best size using the captured `\maxdepth`. Page-builder insert state is an
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
  bytes/ids, not live node handles.
- The implemented stomach shipout path consumes the same box syntax as TeX's
  box primitives (`\shipout\hbox{...}`, `\shipout\boxN`, `\shipout\copyN`),
  traverses the box tree in node order, fires deferred stream whatsits, expands
  deferred-write token lists through the ordinary gullet, serializes the
  `tex-out` artifact, and commits it through `Universe::commit_shipout`, which
  stores the artifact bytes, flushes the committed effect prefix, releases
  shipout-local epoch nodes, and takes the next checkpoint as one boundary.
  Deferred `\openout` and `\closeout` whatsits append the same World stream
  records as `\immediate` stream commands, while deferred `\write` appends the
  routed stream-write record after shipout-time expansion. The same lowering
  traversal carries TeX.web's leader context: deferred stream open, write, and
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
  and font parsing: `Scaled`, physical-unit conversion, `xn_over_d`/
  `nx_plus_y`, `FontSizeSpec`, and TFM fix_word/font-size scaling helpers.
  It has no dependency on state, fonts, or I/O.
- Loading and immutable font-domain data live in `tex-fonts`: TFM for classic
  compatibility; OpenType/TrueType via a
  vendored shaper for the modern path. All file access through `World`
  (fonts are inputs; cross-run memo sharing needs them pinned).
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
- Font parameters are intentionally separate from those immutable metrics.
  `Universe::font_parameter(font, n)` reads the Env-side `\fontdimen` bank, so
  runtime writes are visible to scanners and kernels; the original TFM
  parameter values only seed those banks at load time.
- Later OpenType support should lower backend data behind the same boundary:
  glyph metrics can populate the immutable metric record, while complex
  shaping can replace the TFM pair-program implementation without exposing
  GSUB/GPOS details to paragraph or math kernels.

## 10. Output drivers (`tex-out`)

Responsibility: page artifacts → bytes on disk. Strictly downstream.

- Drivers consume committed page artifacts only — they can run
  out-of-process, in parallel with typesetting of later pages, or not at
  all (editor preview may rasterize page artifacts directly).
- `tex-out` owns the page artifact model and binary reader/writer. It has no
  dependency on `tex-state` or `Universe`; shipout code lowers live state into
  artifact bytes before asking `World` to store them.
- The artifact record captures the effective job magnification and banner at
  shipout, so DVI preamble generation does not reach back into live state.
- PDF driver owns the PDF object model; `\pdfliteral`-class primitives
  produce *effect-log entries* engine-side that the driver interprets —
  the engine never constructs PDF syntax.
- DVI driver is the conformance driver: byte-comparable against Knuth's
  `tex` for the parity corpus. The implemented DVI layer writes the file
  container structure (`pre`, page `bop`/`eop`, first-use `fnt_def`, `post`,
  `post_post`, and 223 padding) from committed artifacts, and traverses the
  committed box tree with TeX.web-style `hlist_out`/`vlist_out`, `movement()`
  w/x/y/z optimization, font switches, rules, glyphs, and DVI specials.
  DVI font numbers are the driver-visible TeX font numbers derived from
  `FontId` load order, not artifact-local dense renumbering, so INITEX parity
  cases that load several sizes/families preserve reference font selection
  bytes.
  The `umber run file.tex --dvi out.dvi` CLI path is a thin downstream
  composition over shipped artifact ids: it reads committed artifact bytes
  from `World`, parses them as `tex-out` page artifacts, and invokes the DVI
  writer without reaching back into live `Universe` state. The DVI parity
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
  the last snapshot before the edit point, re-execute, compare
  `state_hash` at checkpoints; on match, splice the previous run's suffix
  of page artifacts and stop.
  The hash is a semantic checkpoint hash, not a store-layout checksum:
  content handles are followed to token/glue/node/macro contents, control
  sequences are keyed by name, and checkpoint hashes are combined from the
  previous checkpoint plus the current semantic slice.
  A matching checkpoint is directly restartable only when its metadata says
  it is resume-valid. A hash-only checkpoint can still prove convergence, but
  any execution resume must fall back to the checkpoint's recorded
  resume-valid boundary and replay the intervening nested continuation. That
  fallback is separately marked direct-rollback-available or unavailable: a
  nested hash-only shipout that commits effects may drop the `World` effect
  prefix containing the fallback snapshot, in which case incremental drivers
  must restart from an earlier retained checkpoint or replay from a larger
  root instead of rolling directly to the fallback.
- **Memoization**: keyed by (input span or token-list id, read-set epochs,
  code-table generations); value = (journal redo slice, effect slice,
  artifact ids). First target is box/paragraph-level (M4). The memo store
  is content-addressed and shareable across runs and machines.
- **Speculative parallelism**: fork = clone a rolled-back `Universe` onto
  another thread (`Send`, no shared internals — `core_state.md` §10.6);
  speculate page N+1 while N typesets; validate with read-set ∩
  write-set; commit or discard. No pipeline stage knows this is happening.
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

1. **No hidden state.** If a subsystem holds anything mutable across
   tokens, it is either (a) inside `Universe`, (b) summarized in the
   snapshot tuple (input stack, mode nest, page-builder scalars, condition
   stack), or (c) a pure cache validated by epochs/generations (lexer fast
   path, JIT code). There is no (d).
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
| P7 | Memoization, speculation, JIT baseline | M4/M5 |

The guiding rule, as in the state plan: every guard and every piece of
bookkeeping a later phase needs (generations, epochs, read-sets, artifact
hashes) already exists — earlier phases must not invent private shortcuts
that a later phase has to unwind.
