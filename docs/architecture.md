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
  frozen content only through `Stores::tokens`; source reopening by content
  hash remains future `World` integration and is deliberately outside
  `tex-lex`'s local `InputSource` trait. `\endinput` is represented as a
  source-frame flag that lets the lexer finish the current normalized line and
  then pop that source without asking expansion to manage source internals.

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
  through `Stores`, but it does not assign meanings; the stomach/future
  assignment layer remains responsible for installing the returned
  `MacroMeaning`.
- **Numeric value scanning**: `tex-expand::scan_int` and
  `tex-expand::scan_dimen` own the reusable expanded integer and dimension
  scanners for conditionals and the later stomach assignment layer. They pull
  through `get_x_token`; the integer scanner understands TeX integer
  constants and currently readable integer-like state (`\count`, chardef
  values, `\endlinechar`, and raw-sp `\dimen` coercion), while the dimension
  scanner parses decimal constants, physical units, `true` units, supported
  internal dimensions, and opt-in integer-to-sp coercion. Both report
  recoverable numeric diagnostics without performing assignments. Font-relative
  `em`/`ex` units remain explicit TODO stubs until font metrics exist.
- **Conditionals** are a frame-kind, not a side stack: `\if...` evaluation
  marks the frame; `\else`/`\fi` skipping is a token-level scan that the
  fast lexer can accelerate (skip mode only needs catcode classes for
  `\`-detection). The condition stack is part of `InputSummary`.
- **`\csname`** interns through the same interner; **`\the`/`\showthe`**
  read `Env` and mint fresh frozen token lists.
- **Read-set recording** hooks live here and in the stomach: when the
  incremental engine asks for it, meaning lookups record `(cell, epoch)`
  pairs (`core_state.md` §9). Off by default, zero-cost when off (the
  recorder is a generic parameter of the loop, monomorphized away).
- The implemented `tex-expand` scaffold exposes that loop over
  `tex-lex::InputStack` with `Stores` access for meaning reads and explicit
  token-list freezing during macro argument matching. Macro body replay uses
  the body `TokenListId` directly plus frozen argument ids on the replay
  frame; it does not allocate a substituted body list. Token-list replay is
  naturally read-only; source-frame replay can scan already-interned control
  sequence names and reports a lexer error if a source token would require
  minting a new symbol. `\csname` uses a dedicated expansion scan that stops
  on `\endcsname`, validates that expanded name material is character tokens,
  and interns/relaxes the resulting control sequence through an explicit
  sanctioned capability rather than widening expansion to mutable `Env`
  access.
- Frame-control expandables are represented as input-frame rewrites:
  `\expandafter` saves one raw token, performs one expansion step on the
  following token, then pushes the saved token above the expansion result;
  `\noexpand` pushes a one-token replay frame that suppresses expansion for
  exactly the next `get_x_token` read. This keeps suppression frame-local and
  avoids mutating `Env`.
- Value-rendering expandables (`\string`, `\number`, `\romannumeral`,
  `\meaning`, and the currently supported `\the` classes) mint their visible
  output through the explicit token-list freezing capability. `\the` currently
  covers count, dimension, token registers, `\endlinechar`, and `\escapechar`;
  glue-like values, font dimensions, code-table values, box dimensions, page
  state, and time/job parameters remain documented TODOs until those Env
  classes are semantically available.
- Input/job expandables use explicit driver hooks: `tex-expand` scans the
  `\input` file name and asks the caller for a new `InputSource`, while
  `\jobname` renders the caller-provided job name. This preserves the rule
  that file access belongs to `World`/the driver layer, not to the gullet.
  `\fontname` and the mark-family expandables are documented empty stubs until
  font meanings and page-builder marks exist.
- **What the gullet never does**: mutate state. `\def`, `\advance`,
  register writes are *unexpandable* — they are delivered to the stomach.
  This is TeX's own factoring and we enforce it in the crate split:
  `tex-expand` gets `&Env` (plus token-builder access), never `&mut Env`.
  The handful of true exceptions (`\csname` interning, alignment's
  `\span`-time expansion) are threaded explicitly, not by widening the
  borrow.

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
- **Box building**: `\hbox{...}` etc. push a mode level; on close, the
  finished list freezes into the epoch arena; storing it in a box register
  is the barriered promotion write. The stomach never holds a raw node
  pointer across a state write.
- **Paragraph and page hand-off**: when horizontal material ends (`\par`),
  the stomach hands the current list to the paragraph kernel and appends
  the resulting vertical material; the page builder (§8) observes appends
  to the main vertical list.
- The stomach is the *only* pipeline stage holding `&mut Universe`, and it
  holds it as a plain argument — re-entrancy (e.g. `\output` routines,
  `\vsplit`-triggered mark extraction) is recursion in Rust, with the mode
  nest making it explicit and snapshot-summarizable.

## 7. Typesetting kernels

Responsibility: the pure algorithms — node lists in, node lists out. These
are deliberately **libraries, not stages**: they own no state, do no I/O,
and read `Universe` only for parameters and fonts. That purity is what
makes box-level memoization (M4) sound.

- **Paragraph builder / line breaker**: Knuth–Plass over a frozen
  horizontal list. Hyphenation (patterns loaded via `World`, compiled into
  an immutable trie), ligature/kerning via `tex-fonts`. Output: vertical
  list of hboxes + penalties/glue. Parameters (`\tolerance`, `\parshape`,
  ...) are read once at entry into a plain struct — the kernel never
  touches `Env` mid-algorithm, which keeps its read-set a clean prefix.
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
  tracking active), the effect-log prefix flushes, and old history drops.
- Page artifacts are the currency between the engine and both the output
  drivers and the incremental engine: a page artifact = (serialized node
  tree, resources used (fonts/images by content hash), effect slice).

## 9. Fonts and metrics (`tex-fonts`)

Responsibility: every question about glyphs, loaded once, answered from
immutable tables.

- Loading: TFM for classic compatibility; OpenType/TrueType via a
  vendored shaper for the modern path. All file access through `World`
  (fonts are inputs; cross-run memo sharing needs them pinned).
- A loaded font is an immutable object; `FontId` is the state-layer handle
  (`core_state.md` §10.3) minted at load time; `\font` assignment is an
  ordinary barriered `Env` write. Per-font mutable parameters
  (`\fontdimen`) live in `Env`-side banks, *not* in the font object —
  loaded fonts stay immutable and shareable across snapshots and threads.
- Shaping sits behind a kernel-facing API (`shape(FontId, &str|glyph run)`)
  so the paragraph builder does not know which backend answered.

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
   not an I/O call.
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
