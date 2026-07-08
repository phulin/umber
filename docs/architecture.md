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
  barrier. The pipeline stages hold **no hidden state of their own** beyond
  what the snapshot tuple captures (input stack summary, stream buffers —
  see `core_state.md` §9). That is the invariant that makes a snapshot
  sufficient to resume anywhere.

## 2. Crate map

```text
umber (CLI / driver)
 ├── tex-incr        incremental engine: convergence, memo store, speculation
 │    └── tex-engine       the interpreter proper
 │         ├── tex-expand       gullet: macros, conditionals, expandable prims
 │         ├── tex-exec        stomach: mode machine, unexpandable prims
 │         ├── tex-typeset    par builder, line break, math, alignment, page
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
  `\else`), whether the current and any previous limb has been taken, the
  `\ifcase` `\or` count, and the nested conditional depth observed during
  skip/resume scanning.
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
  token-list frame and continue; else deliver it downstream.
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
  without performing assignments. `true` physical units call the
  `Universe::prepare_mag` boundary before scaling, so illegal magnifications
  are coerced and the job-level magnification is frozen in snapshot-covered
  state for later shipout/font paths. Font-relative `em`/`ex` units remain
  explicit TODO stubs until font metrics exist (umber2-flt).
- **Conditionals** are a frame-kind, not a side stack: `\if...` evaluation
  marks the frame; `\else`/`\fi` skipping is a token-level scan that the
  fast lexer can accelerate (skip mode only needs catcode classes for
  `\`-detection). The condition stack is part of `InputSummary` and carries
  limb/taken state, `\ifcase` `\or` count, and skip nesting so rollback can
  restore an open conditional without reconstructing hidden gullet state.
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
  diagnostics.
- Value-rendering expandables (`\string`, `\number`, `\romannumeral`,
  `\meaning`, and the currently supported `\the` classes) mint their visible
  output through the explicit token-list freezing capability. `\the` covers
  integer, dimension, glue, muglue, and token registers; register aliases;
  integer, dimension, glue, and token parameters represented in `Env`; and
  code-table values. Font dimensions, box dimensions, page state, and
  time/job parameters not yet backed by `Env` remain documented TODOs until
  those classes are semantically available.
- Input/job expandables use explicit driver hooks: `tex-expand` scans the
  `\input` file name and asks the caller for a new `InputSource`, while
  `\jobname` renders the caller-provided job name. This preserves the rule
  that file access belongs to `World`/the driver layer, not to the gullet.
  `\fontname` and the mark-family expandables are documented empty stubs until
  font meanings and page-builder marks exist.
- The stomach implements the macro-definition assignment surface used by the
  expansion conformance path: `\def`, `\edef`, `\gdef`, `\xdef`, `\let`,
  `\futurelet`, prefix accumulation (`\global`, `\long`, `\outer`,
  `\protected`), and `\globaldefs` override behavior. These commands scan
  through the shared gullet/token scanner where expansion is required and
  write meanings only through the barriered `Universe` facade. The
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
- **Box building**: `\hbox{...}` etc. scan a packing spec, execute a nested
  restricted-horizontal or internal-vertical list builder, freeze the
  finished list into the epoch arena, then call the pure `tex-typeset`
  packing kernel. Storing the resulting one-node list in a box register is
  the barriered promotion write. Pulling boxes back out through `\copy`,
  `\box`, unboxing, `\lastbox`, or box-dimension rewrites clones any
  survivor-backed node tree into the current epoch before it can be appended
  to an unfinished mode list or promoted again. Horizontal list construction
  buffers adjacent font-backed characters in the stomach until a boundary
  command, then reconstitutes them through the loaded font's TFM ligature/kern
  program, updates the mode-local `\spacefactor`, and appends explicit
  h-mode nodes for spaces, kerns, skips, finite-fill glue, penalties, rules,
  discretionaries, accents, and italic corrections. Paragraph breaking is a
  hand-off to the pure `tex-typeset` line breaker; page contribution remains
  a later hand-off.
  Vertical list construction tracks TeX's `prev_depth` on each mode-list
  level. A single shared append routine handles every box or rule appended to
  vertical/internal-vertical lists, including explicit box appends, unboxed
  vlist children, and paragraph lines. It inserts the implicit adjusted
  `\baselineskip` glue, or `\lineskip` when the adjusted baseline glue is
  below `\lineskiplimit`, unless `prev_depth` is TeX's ignore sentinel. This
  keeps baseline/interline side effects in the stomach boundary; `tex-typeset`
  receives explicit glue nodes and remains a pure measurement and packing
  kernel.
- **List diagnostics**: `\showbox` routes through `World` terminal/log
  effects and uses the shared node-list dump emitter in `tex-exec`. The
  emitter walks frozen node lists through `Universe`, honors
  `\showboxbreadth` and `\showboxdepth`, and is intentionally reusable by
  future `\showlists` and `\tracingoutput` diagnostics rather than tied to
  `\showbox` scanning.
- **Paragraph and page hand-off**: paragraph start/end is stomach-owned.
  `\indent`, `\noindent`, implicit start from vertical-mode character
  material, `\parskip`, and `\everypar` replay are handled before entering
  unrestricted horizontal mode. When horizontal material ends (`\par` or
  `\endgraf`), the stomach performs TeX's final paragraph-list preparation
  (trailing-glue removal and `\penalty10000` plus `\parfillskip`), snapshots
  paragraph-shape and line-breaking parameters, calls the pure line breaker
  over the prepared hlist, runs separate post-line-break surgery, freezes each
  resulting line list, hpack's it to the captured line width, and appends the
  hboxes through the shared vertical append routine. The page builder (§8)
  observes appends to the main vertical list.
- The stomach is the *only* pipeline stage holding `&mut Universe`, and it
  holds it as a plain argument — re-entrancy (e.g. `\output` routines,
  `\vsplit`-triggered mark extraction) is recursion in Rust, with the mode
  nest making it explicit and snapshot-summarizable.
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
  routes through `tex-typeset`; full page contribution remains future work.

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
  mid-algorithm. The current `tex-exec` integration precomputes the
  hyphenated hlist because automatic hyphen insertion still freezes
  discretionary child lists through `Universe`; the breaker only sees the
  hook result as ordinary nodes. Post-line-break produces line node vectors
  with `\leftskip`/`\rightskip` and interline penalty decisions; the stomach
  remains responsible for freezing those vectors, hpacking to the captured
  width, and appending hboxes to the enclosing vertical list. Remaining
  pdfTeX corpus parity details are tracked as follow-up work rather than
  weakening this purity boundary.
- **Math typesetter**: mlist → hlist conversion, styles, fraction/radical
  layout, math fonts. Same contract: frozen mlist in, frozen hlist out.
  (OpenType MATH is the target metrics model; TFM math as fallback.)
- **Alignment (`\halign`/`\valign`)**: the one kernel that is *not* pure —
  template expansion interleaves with the gullet by design. It is
  structured as a stomach sub-mode (it re-enters main control per cell),
  not as a kernel function, and is therefore excluded from kernel-level
  memoization (page-level still covers it).
- **Vertical packing, `\vsplit`, marks**: operate on survivor-arena lists
  (they are reachable from box registers by definition); mark extraction
  reads are recorded like any state read.
- **Implemented packing foundation**: `tex-typeset` currently provides pure
  `hpack`, `vpack`, `vtop`, and TeX.web §108 badness over frozen node lists.
  The crate reads `Universe` immutably, including frozen nodes, glue specs,
  and loaded font character metrics, copies packing parameters into plain
  structs at entry, and returns box payloads plus diagnostics without writing
  state. Stomach-side box-building primitives live in `tex-exec`; the packing
  crate remains pure and has no `World` or `&mut Universe` surface.

## 8. Page builder and output routine

Responsibility: accumulate the main vertical list, fire `\output`, commit.

- The page builder is an incremental observer of main-vertical-list
  appends (contributions), maintaining `\pagegoal`/`\pagetotal` and the
  best-break record, inserting from `\insert` classes. It is stomach-side
  state, summarized in snapshots (it is small: a handful of scalars + the
  contribution list, which is an ordinary unfrozen node list).
- **`\output` is a recursion**: box 255 is filled, the output routine's
  token list replays as a frame, `\shipout` is a primitive delivered back
  to the stomach. Nothing special architecturally — except that
  **`\shipout` is the commit barrier** (`core_state.md` §9): the shipped
  page freezes, serializes into the content-addressed artifact store,
  deferred `\write`s expand *now* against current state (with read-set
  tracking active), the effect-log prefix flushes exactly once through
  `World`, and old history drops.
- Page artifacts are the currency between the engine and both the output
  drivers and the incremental engine: a page artifact = (serialized node
  tree, resources used (fonts/images by content hash), effect slice).

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
- PDF driver owns the PDF object model; `\pdfliteral`-class primitives
  produce *effect-log entries* engine-side that the driver interprets —
  the engine never constructs PDF syntax.
- DVI driver is the conformance driver: byte-comparable against Knuth's
  `tex` for the parity corpus.
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
4. **Effects only via `World`; commit only at shipout.** Any new
   primitive with an observable side effect gets an effect-log entry kind,
   not an I/O call. Uncommitted records are rollback state; committed
   prefixes are materialized through `World` and then discarded from the
   in-memory log.
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
