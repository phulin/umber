# Core Engine State — Design Plan

Status: draft for implementation
Scope: the state layer (`tex-state` crate) of a modern TeX engine — storage,
mutation discipline, snapshots, and the enforcement architecture.
Out of scope: expansion/typesetting algorithms, JIT codegen strategy, output
drivers. Those are *consumers* of this layer and are referenced only where
they constrain it.

---

## 1. Goals and non-goals

The state layer must simultaneously serve four consumers:

1. **Interpreter throughput.** State reads are the hottest operations in the
   engine (meaning lookup per control sequence; code-table lookup per input
   character). Reads must be one indexed load from flat memory. The write
   barrier must be a few straight-line, branch-predictable instructions.
2. **A future JIT.** Compiled code needs stable cell addresses for the
   lifetime of the process, and per-cell version stamps usable as inline-cache
   / deoptimization guards.
3. **Snapshots and rollback.** Taking a checkpoint must be O(1); we take one
   per shipped page and (while interactively editing) per paragraph. Rolling
   back must cost proportional to what changed, not to total state size.
4. **Memoization, convergence detection, and speculative parallelism.**
   These must be expressible as *queries over existing bookkeeping* (read
   sets from epoch stamps, write sets from the journal, effect replay from
   the effect log) — not as separate instrumentation.

Non-goals: bit-compatibility with Knuth's memory layout (we keep the
*semantics*, including grouping and `\global`, not the representation);
supporting untracked mutation "for performance" anywhere, ever.

### Design principles

- **Separate identity, meaning, content, and history.** Each gets the
  representation its access pattern wants.
- **Mutable state in flat arrays; immutable content in hash-consed arenas;
  history in one append-only journal.** Persistence is a property of data
  that is genuinely persistent; arrays for data that is genuinely mutable.
- **One write barrier.** Every semantic mutation flows through a single
  method on a single struct that owns its own history. The journal *is* the
  write-set; epoch stamps *are* the read-set timestamps; every advanced
  feature is a query over these.
- **Completeness over cleverness.** The lesson of prior art (SwiftLaTeX's
  in-engine checkpointing, which "breaks certain projects"): any state not
  captured by the snapshot machinery is a future heisenbug. Enumerate
  everything; virtualize all effects; verify dynamically (§9).

---

## 2. Store overview

| Store | Contents | Mutation discipline | Snapshot mechanism |
|---|---|---|---|
| Interner | csnames, key strings → `Symbol` | append-only | watermark |
| Environment | meaning word + epoch per symbol; parameters; dense registers; current-font and math-family font selectors | barriered in-place writes | journal |
| Register overflow | e-TeX sparse registers (256..32767) | barriered writes | journal + page roots |
| Code tables | catcode/lccode/uccode/sfcode/mathcode/delcode over Unicode | copy-on-write pages | root pointer + generation |
| Token store | immutable, hash-consed token lists | frozen at birth | watermark |
| Glue store | immutable, hash-consed glue specs (`GlueId`) | frozen at birth | watermark |
| Node arenas | per-epoch bump arenas + survivor arena | frozen at birth; promotion on escape | watermark; refcounts (survivors) |
| Hyphenation | language-0 Liang trie + exception map | pattern/exception loads through `Universe` | cloned in store snapshot (v1) |
| Journal | undo records + group/checkpoint markers | append-only | position |
| Effect log | deferred writes, aux/toc/idx, shell escape, PDF objects | append-only, committed at shipout | position + stream buffers |
| Page builder | contribution list, current page, `page_so_far` dimensions, page integers, ordered per-class insertion records, best break/fire-up records, five TeX82 mark token-list slots | mutation through `Universe` page facade | copied into snapshot tuple + semantic hash |
| Misc scalars | RNG state, interaction mode, current epoch, prepared magnification, input-stack summary | barriered / snapshot-owned | copied into snapshot tuple |

A **snapshot is a tuple of positions and roots** into these stores — a few
dozen words, O(1) to take (§7).

Snapshots also carry checkpoint metadata for incremental consumers. A
checkpoint may be **resume-valid**, meaning the executor was at a quiescent
boundary with no hidden Rust-stack continuation, or **hash-only**, meaning
the store/world tuple is valid for rollback and convergence hashing but not
for direct execution restart. Hash-only checkpoints record the previous
resume-valid checkpoint id/hash as their resume fallback. The fallback also
records whether direct rollback is still available under bounded effect
history. If a commit has dropped the needed `World` effect prefix, the
checkpoint remains useful for convergence but the driver must restart from an
earlier retained boundary or replay from a larger root rather than treating
that fallback as rollback-ready.

---

## 3. Identity: the interner

- All names intern to dense `Symbol(u32)`. Backing: bump-allocated UTF-8
  arena + open-addressing hash index.
- Append-only: nothing is un-interned. Rollback = truncate arena to
  watermark. The hash index either records insertion order (rewindable) or
  is rebuilt lazily on first intern after a rollback — interning after
  rollback is rare, so lazy rebuild is acceptable v1.
- Dense ids make every downstream lookup an array index; stable ids are what
  compiled code embeds.
- `\csname`-manufactured names go through the same interner (expl3 does this
  constantly; the interner must be fast and its growth must be watermarked
  like everything else).

## 4. Meaning: the environment

Successor to the eqtb. Struct-of-arrays keyed by `Symbol`:

```rust
// One 64-bit meaning word per symbol.
// opcode : 8   — dispatch case ("macro", "\relax", "chardef", "font", ...)
// flags  : 8   — \long, \outer, \protected, frozen
// operand: 48  — TokenListId | FontId | char+catcode | register index | ...
pub struct Env {
    cells:   Box<[u64]>,       // meaning words
    epochs:  Box<[Epoch]>,     // parallel stamp per cell (Epoch = u32/u64)
    // dense classical registers, one bank per class:
    counts:  Box<[u64; 256]>, dimens: Box<[u64; 256]>,
    skips:   Box<[u64; 256]>, toks: Box<[u64; 256]>,
    boxes:   Box<[u64; 256]>,
    reg_epochs: /* parallel stamps per bank */,
    int_params: Box<[u64; N_INT]>,   dimen_params: Box<[u64; N_DIM]>,
    glue_params: Box<[u64; N_GLUE]>, tok_params: Box<[u64; N_TOK]>,
    overflow: SparseRegisters,   // e-TeX 256..32767, radix pages, mostly empty
    journal: Journal,
    epoch: Epoch,
}
```

Rules:

- **Reads**: `get(&self, Symbol) -> Meaning` — one load, decode in
  registers. Dense register banks and parameter tables follow the same
  discipline: storage is raw `u64` words, and typed accessors encode/decode
  `i32`, `Scaled`, and content ids at the API boundary. Meaning words and
  register values are `Copy`; no references into the array escape. Character
  token meanings preserve both character and category code so `\let`
  aliases such as `\let\x=&` retain delimiter meaning where TeX requires
  command-code comparisons. Unknown opcode words decode through a
  crate-private stored-word codec into an opaque raw-meaning value whose
  fields can be read and re-encoded after `Env::get`, but downstream crates
  cannot decode arbitrary raw words or construct arbitrary raw meanings in
  production builds.
- **Writes**: symbol-keyed meaning writes are exposed through the owning
  `Universe` facade, which validates that the `Symbol` is live in the
  same interner timeline before calling Env's crate-private barriered setter.
  Same for every register bank and parameter table: semantic assignment runs
  the barrier (§6). TeX's named math glue parameters (`\thinmuskip`,
  `\medmuskip`, and `\thickmuskip`) use distinct muglue meanings in the
  command table so scanners accept `mu` units, but their values are still
  journaled glue-parameter cells keyed by TeX's parameter indices. Journal
  restore walks use a crate-private
  `Env::restore_raw(CellId, u64)` primitive that bypasses the barrier; it is
  restore-only, not a semantic assignment API.
- **Math glue provenance**: immutable glue nodes include diagnostic-only
  variants for explicit `\mskip` and converted `\thinmuskip`, `\medmuskip`,
  and `\thickmuskip` math spacing. These variants preserve `\showbox` /
  `\showlists` labels after mlist conversion; shipout lowering treats all of
  them as normal glue so serialized page artifacts do not gain DVI-visible
  distinctions.
- **Leader glue payloads**: `\leaders`, `\cleaders`, and `\xleaders`
  attach their scanned box or rule payload directly to the glue node. The
  payload is immutable node content, not a side table keyed by `GlueId` or
  node span, so survivor promotion, rollback/replay, and semantic state
  hashing follow the ordinary node-list ownership rules.
- **Public boundary**: downstream crates may read `Env` through the owning
  aggregate, but they cannot construct or mutate a standalone `Env`; raw Env
  construction, group control, epoch advancement, and typed setters are
  crate-private or test-only.
- **The epoch stamp is the workhorse**: journal coalescing filter, JIT
  inline-cache guard, and memoizer read-set timestamp. One counter, three
  consumers. Do not add a second versioning scheme for any of these.
- Macro bodies are **not** stored here. A macro meaning word stores opcode
  `macro`, the public flag byte (`\long`, `\outer`, `\protected`, plus the
  reserved frozen bit), and a 48-bit operand naming an immutable
  `MacroDefinitionId`. That definition is owned by the aggregate
  `Universe` boundary and contains two frozen token-list ids:
  parameter text and replacement text. Downstream code may decode macro
  meanings through public aggregate facades into the `MacroMeaning` aggregate,
  but it cannot mint live macro-definition ids or inspect the raw store.
  Identical token lists are hash-consed by the token store, so separately
  scanned identical replacement bodies receive the same `TokenListId`; macro
  definitions themselves are also hash-consed over flags, parameter text id,
  and replacement text id.
- Addresses of `cells` are stable for the process lifetime (no reallocation:
  size the array to the interner's max, grow by chunked segments if needed —
  segments never move).

### Register overflow (e-TeX sparse tier)

Registers 256..32767 per class: two-level radix pages (page = 256 slots),
default-page shared, cloned on first write, entries barriered like dense
registers. This unifies what e-TeX bolted on as separate save-stack
machinery: **one write path for all registers**.

## 5. Meaning, sparse tier: the code tables

Six tables over ~1.1M codepoints; overwhelmingly default-valued; writes are
rare and bursty (verbatim, `\makeatletter`, babel shorthands).

- Representation: two-level paged radix tree; root of page pointers →
  pages of 256 entries. All-default pages alias canonical shared constants.
- **Copy-on-write at page granularity**: first write to a shared page clones
  it. Old snapshots keep the old root; no journaling of individual entries
  needed (the root swap is the undo record — the journal stores old roots).
- Each table carries a **generation counter**, bumped on any write. The
  lexer's SIMD fast path is compiled/validated against a generation vector
  and never touches the tree until a generation bump forces reclassification.
  This is the storage-level grounding of catcode speculation. Generations
  represent assignment activity, not effective value changes: a same-value
  code-table assignment still bumps the table generation, though it need not
  copy a page because the table content is unchanged.
- In the implemented `tex-state` API, code tables live behind `Universe`:
  reads and writes go through `Universe::{catcode,set_catcode,...}` and
  `Universe::code_table_generations`. `Universe::snapshot` captures the
  structurally shared root pointers and generation counters, and
  `Universe::rollback` restores them atomically with the Env/content tuple.
  Each implemented table root starts at a canonical shared default root whose
  pages are also canonical shared pages; the first effective write detaches
  the root and then copy-on-writes only the touched page.
- INITEX defaults are TeX82-compatible for the classic 0..255 range and
  extended over Unicode by the same default rules: ASCII letters have letter
  catcode and case mappings, uppercase ASCII has `sfcode` 999, other scalar
  values keep the normal TeX defaults (`other` catcode, zero case codes,
  `sfcode` 1000, mathcode equal to the scalar value, delcode -1) until set.
- Rationale for structural persistence *here only*: the read path that
  matters bypasses the tree; the domain is huge and default-dominated; and
  per-snapshot roots make history free. Everywhere else, flat arrays win.

### Hyphenation state

TeX82's hyphenation tables are represented in `tex-state` as a language-0
Liang trie plus an exception map. Pattern loading normalizes letters through
the current `\lccode` table, stores digits as inter-letter hyphen weights,
and treats `.` as the word-boundary edge. The trie is immutable from the
consumer perspective: execution primitives append patterns/exceptions through
the `Universe` facade, while hyphenation lookups read a stable table and
produce plain character positions.

The v1 snapshot strategy clones the hyphenation table into `StoreSnapshot`.
This keeps rollback and replay semantics exact without exposing raw store
internals or adding a second ad hoc journal. It makes snapshots after pattern
loads proportional to the table size; that is acceptable while patterns are
loaded INITEX-style at job start. A future multi-language/e-TeX pass can
replace this with a persistent arena or table journal if format-sized pattern
sets make checkpoint cost visible.

## 6. History: the journal and the write barrier

One append-only log for all barriered state:

```rust
struct UndoRec { cell: CellId, old: u64 }   // 16 bytes; CellId spans all banks
enum Marker { Group, Checkpoint(SnapshotId) }
```

**Barrier** (the same ~5 instructions whether emitted by `Env::set` or by
the JIT):

```text
if epochs[i] < current_epoch:
    journal.push(UndoRec { cell: i, old: cells[i] })
    epochs[i] = current_epoch
cells[i] = new
```

- **First-write-per-epoch coalescing**: journal growth is bounded by
  distinct cells touched per epoch (typically a few hundred per page,
  single-digit KB), not by write count.
- **No-op local writes do not consume the epoch**: assigning the word already
  in the cell skips the local barrier record and leaves the stamp alone, so a
  later same-epoch real change still records its pre-change value. A same-word
  `\global` assignment still records a global undo record, because it can
  change which group owns the value even when the current word is unchanged.
- **Undo+redo coalescing note for M1**: because only the first write to a
  cell in an epoch appends an `UndoRec`, the record's `new` value is the
  first written value and may be stale after later same-epoch writes. This is
  intentional for M1: rollback consumes `old`, and future redo-replay /
  memo consumers must re-derive final values from the live cells when they
  need them.
- **Groups are journal markers.** This *replaces* Knuth's save stack: group
  entry pushes `Marker::Group` and bumps the epoch; group exit walks back to
  the marker restoring records. Same records, same log, one mechanism.
- **`\global` is logged too**, tagged so group-exit restoration skips it and
  compacts it below the marker (the analogue of e-TeX's sparse-register
  compaction). "Survives the group" and "survives rollback to page 12" are
  different lifetimes; only the journal serves the second.
- Epoch bumps happen at: group entry, group exit, checkpoint, and
  (optionally) paragraph boundaries while interactive. Monotonic; never
  reused within a session. The group-exit bump is load-bearing: restoration
  rolls values back but deliberately leaves epoch stamps high, so without a
  fresh epoch the next write to a restored cell would match the current
  stamp, skip its journal push, and silently corrupt the *enclosing*
  group's undo slice.

## 7. Content: token store and node arenas

### Token store

- Token lists are **immutable after construction** (hard invariant; Knuth
  already ref-shares macro bodies — we make it structural).
- Built via builder-then-freeze (§8.4); on `finish()`, **hash-consed**:
  identical lists get identical `TokenListId`s. Benefits: `\ifx` on bodies is
  an id compare; memo keys are hashes; identical expansions share storage
  across snapshots automatically.
- Backing: bump arena + hash index; rollback = watermark truncation + lazy
  index repair (same policy as interner).

### Glue store

- Glue specs are immutable five-field values: width, stretch amount and order,
  shrink amount and order. `GlueId` is hash-consed over those fields, so equal
  specs share identity across snapshots and field-order differences remain
  observable.
- `GlueId::ZERO` is the canonical pre-interned zero-glue spec in every store.
  Watermark truncation has a floor at that canonical entry, so rollback never
  removes it.

### Node arenas

- Nodes are born into a **per-epoch bump arena**. The overwhelming common
  case — node dies within its page — is freed by arena truncation (rollback)
  or wholesale release (after shipout). No free lists, no tracing GC.
- In M2, the epoch arena is one growing `Vec<Node>` plus immutable
  `NodeListId { arena, start, len }` spans minted only by
  `NodeListBuilder::finish(&mut NodeArena)`. Builders are owned scratch
  buffers; finishing appends and clears them. Child lists inside newly-frozen
  epoch nodes must already be frozen lower in the same epoch arena; debug
  builds assert this bottom-up discipline. Survivor ids name a root slot plus a
  root-relative span and are read through the aggregate-owned survivor arena.
  Survivor refcounts are owned incrementally by live box registers plus
  retained box-register journal records. Appending a box undo record claims
  its old survivor value, and truncating a walked journal slice releases the
  corresponding owner, so survivor liveness follows the same O(changed-slice)
  boundary as rollback and group exit.
- **Promotion on escape**: storing a node list into a box register, mark,
  or insertion is a barriered write, so the engine sees it and copies the
  list into a **survivor arena** with per-box refcounts. The promoted root is
  one contiguous allocation; child `NodeListId`s are remapped to root-relative
  survivor spans by an explicit worklist, so deeply nested boxes do not
  recurse through the Rust call stack during promotion. `\unhbox`/`\vsplit`
  operate on survivors. The rare escaping box pays a copy; every epoch arena
  earns the right to be truncated blindly. Test-only replay/hash helpers that
  still walk node trees recursively carry explicit depth bounds until M3
  replaces them with convergence-grade semantic hashing.
- **Returning from survivor storage**: unfinished mode lists are epoch-owned,
  so a box copied or removed from a survivor-backed register is cloned back
  into the current epoch before append, unbox, dimension rewrite, or
  re-promotion. This preserves the bottom-up invariant for epoch node lists
  while keeping box-register storage survivor-owned and journal-accounted.
- Shipped pages serialize into content-addressed artifacts (the memo/extern
  store) and their nodes are released.

## 8. External effects: the virtualized world

Nothing in the engine touches the OS directly. A single `World` object owns:

- **Content-addressed inputs**: `World::read_file` and `\openin` reads
  return bytes plus a stable `ContentHash`, and append an `InputRecord` to
  the snapshot-owned World state. The real backend is the only engine code
  that uses host files; the in-memory backend exposes the same API for
  hermetic tests and corpus drivers. `\read` terminal input is also owned by
  `World`; in-memory worlds keep a replayable terminal line buffer plus a
  snapshot-owned cursor, while real worlds read stdin only through this
  boundary.
- **Output streams** (`\openout`/`\write`, aux/toc/idx): writes append to an
  effect log; stream buffer state *including partial lines* is snapshot
  state. TeX's own defer-`\write`-to-shipout semantics is the model —
  extended to every effect. `World` owns the 16 stream slots, terminal/log
  sinks, partial-line buffers, terminal-input cursor, and an append-only effect log. `\openout`,
  `\closeout`, routed stream writes, special-class payloads, PDF object
  placeholders, and shell-escape requests append records only; no host bytes
  materialize until the commit barrier flushes a prefix. Immediate output
  commands append these `World` records at execution time and rely on the
  final job commit unless a shipout commits them earlier; non-immediate
  `\openout`, `\write`, and `\closeout` reach `World` only when their whatsit
  nodes are shipped.
- **Deferred stream whatsits** model non-immediate `\openout`, `\write`, and
  `\closeout` as node-list content, so copied boxes replay them once per
  shipout. Deferred `\openout` whatsits carry the stream slot plus scanned
  target path, deferred `\closeout` whatsits carry the stream slot, and
  deferred `\write` whatsits carry the resolved `PrintSink` plus the
  unexpanded `TokenListId`. Deferred-write token lists are expanded at shipout
  against the state *at the commit barrier*; read-set tracking must therefore
  cover shipout-time expansion, not just mainline execution. Shipout fires
  these whatsits in node order by appending the corresponding stream-open,
  stream-write, and stream-close records immediately before committing the
  page prefix. When any of these whatsits is contained in a leader payload,
  shipout lowering follows TeX.web's `doing_leaders` rule and suppresses the
  deferred stream effect instead of opening, expanding, closing, or anchoring
  it. In contrast, source `\special` expands its balanced text when the
  primitive is scanned and stores detached DVI-class payload bytes in a
  whatsit; shipout anchors that already-expanded payload into the committed
  page effect slice even inside leaders, so the DVI writer can emit `xxx`
  output for each repeated payload.
- **Shell escape, PDF object stream, log file**: same discipline — buffered,
  committed at shipout, discarded on rollback. Shell escapes are record-only
  and the execution policy defaults to disabled.
- **RNG state and clock reads**: owned by `World`, journaled/snapshotted.
  The clock is read once when constructing a real `World`; `Universe` copies
  that job-start clock into `\time`, `\day`, `\month`, and `\year`.
- **Inputs** (file reads) are content-addressed and recorded, so a snapshot
  pins exactly what it read (needed for cross-run memo sharing).
- **Page artifacts** are committed through `Universe::commit_shipout`, which
  stores bytes through crate-private `World` artifact storage as part of the
  aggregate commit boundary. Real worlds materialize those bytes under the
  configured artifact directory; in-memory worlds keep the same
  content-addressed map for hermetic tests.
- **Explicit driver output files** are materialized through `World::write_file`.
  This is for user-requested downstream files such as `umber run --dvi`; engine
  primitives still record effects and rely on the shipout commit barrier.

Effects **materialize only when the producing page commits** (shipout).
Rollback discards the uncommitted suffix of the effect log. Commit accepts an
`EffectPos`, flushes records from the last committed position through that
prefix in order, and then drops the flushed prefix from memory; committing the
same or an older position is a no-op, so each record reaches `World`'s real
backend exactly once. Snapshots older than the dropped prefix must be discarded
by the caller as part of the bounded-history policy. `Universe` reflects this
in checkpoint metadata: a hash-only fallback whose snapshot predates the
retained effect history is marked unavailable for direct rollback instead of
being exposed as a resume-ready boundary.

`World` is storage for external facts, not a public timeline-control object.
Its authority is split conceptually into two downstream-safe capabilities plus
one aggregate-only authority:

- **Read-only world view**: inspect already-recorded facts such as the current
  effect position, pending effect records, input records, committed artifacts,
  stream-buffer summaries, and in-memory backend outputs. This view cannot
  mutate the effect log or commit anything.
- **Operational world I/O**: perform ordinary engine operations that must be
  virtualized by `World`: content-addressed file reads, TeX stream
  open/close/read/write recording, terminal/log effect recording, deferred
  write/special/shell-escape recording, RNG consumption, and test-memory
  seeding where a hermetic backend is in use. This capability may append
  rollback-covered records, but it cannot drop committed prefixes or take
  snapshots.
- **Timeline control**: snapshot, rollback, state-hash cursors, page-artifact
  commit, and effect-prefix commit are aggregate `Universe` operations. Raw
  `World` commit/rollback helpers remain crate-private implementation details,
  because dropping an effect prefix also requires `Universe` to retarget hash
  cursors and advance the aggregate checkpoint.

The public API preserves that split: downstream crates may inspect facts
through `&World` and perform operational I/O through `&mut World`, while raw
effect-prefix commit, artifact storage, snapshots, rollback, and hash cursors
remain crate-private implementation details reached only through aggregate
`Universe` boundaries.

## 9. Snapshots, rollback, commit

```rust
pub struct Snapshot {
    owner: SnapshotOwner,          // rejects cross-Universe misuse
    journal_pos: JournalPos,
    group_depth: u32,              // group exits invalidate enclosed snapshots
    epoch: Epoch,
    watermarks: Watermarks,        // tokens, nodes, strings, interner, aftergroup
    code_roots: [PageRoot; 6],     // + generation vector
    overflow_roots: PageRoots,
    effect_pos: EffectPos,
    stream_bufs: StreamBufState,
    rng: RngState,
    input_stack: InputSummary,     // lexer-owned state needed to resume the mouth
    page: PageBuilderState,        // current page, contributions, page scalars
    state_hash: u64,               // for convergence detection
    checkpoint_id: CheckpointId,
    resume_kind: ResumeValid | HashOnly,
    resume_fallback: Option<ResumeFallback>, // boundary id/hash + direct rollback availability
}
```

- **Take**: O(1) — record positions/roots, copy scalars. Frequency: every
  shipout; every paragraph while an editor session is hot. A snapshot belongs
  to the `Universe` instance that created it. Snapshots taken
  inside a TeX group are valid only while that enclosing group is still open;
  leaving the group truncates the journal below the checkpoint position and
  invalidates those snapshots instead of permitting partial rollback.
- **Resume validity**: `Universe` distinguishes hash checkpoints from
  restartable execution checkpoints. Top-level/quiescent checkpoints are
  resume-valid. Checkpoints taken while the stomach is executing a nested
  continuation whose phase still lives on the Rust call stack are hash-only:
  they advance the semantic checkpoint hash, but their metadata points resume
  to the previous resume-valid boundary and says whether direct rollback to
  that boundary is still retained. Alignment row/cell execution, `\noalign`
  groups, template replay, and box-group scanning use this conservative
  fallback until those continuations are serialized explicitly.
- **Input restoration**: `InputSummary` carries the lexer-owned source-frame
  state required after a source is reopened: source-local offsets, current
  normalized line, in-line char/byte offsets, lexer N/M/S state, queued
  synthetic tokens such as a blank-line `\par`, token-list replay positions,
  macro-body replay argument slots, open condition frames, and the last
  popped source frame. Condition frames are snapshot-owned input frames; each
  carries its conditional family (`\if...` or `\ifcase`), current limb
  (`\if`, `\or`, or `\else`), current/previous taken bits, `\ifcase`
  `\or` count, and skip nesting depth needed to resume token-level skipping.
  Durable source reopen identity is not a `tex-lex` field; it is part of the
  `World` input/effect snapshot that pins file/editor content by content hash
  and recreates the `InputSource` before these frame summaries are applied.
- **Rollback**: replay journal to marker (restoring cells and old code-table
  roots); truncate arenas to watermarks; release survivor owners held by the
  truncated box-register journal records while restored registers reclaim
  their old owners; discard effect-log suffix; restore scalars and the
  Universe-owned page-builder tuple.
  Pending `\aftergroup` payloads are part of the Env rollback tuple: snapshots
  carry an aftergroup length and rollback truncates payloads pushed after the
  snapshot. The epoch counter is never rewound — rollback bumps it past its previous
  maximum (stale high stamps on restored cells would otherwise bypass the
  barrier's journal push, same failure as skipping the group-exit bump).
- **Atomicity rule (hard invariant)**: meaning cells contain content ids, so
  the journal and the arena watermarks restore **as one tuple, never
  independently** — otherwise every box register dangles. Enforce by making
  rollback a single method on the top-level `Universe` (§10.6); no partial
  rollback API exists. In the implementation, `Universe` owns a private
  `Stores` composition for the Env/interner/content/survivor/code-table tuple;
  `Env` journal positions, journal walks, raw rollback, raw root restoration,
  and `Stores` checkpoint/rollback remain crate-private implementation details
  behind `Universe::snapshot`, `Universe::rollback`, and the liveness-checking
  `Universe` write facades.
- **Commit barrier = shipout**: page artifact serialized and stored through
  `World`, effects flushed, shipped-page epoch nodes released, and then the
  next checkpoint taken through one aggregate `Universe::commit_shipout`
  boundary. The flushed effect prefix is dropped too, leaving only the
  uncommitted suffix and the committed backend stream state. History is
  bounded. The implemented boundary retargets hash cursors past dropped effect
  prefixes before checkpointing, so later shipout checkpoints never try to hash
  already-committed effect records or released page-local nodes. If shipout
  occurs while a hash-only stomach continuation scope is active, this commit
  checkpoint is also hash-only and records the previous resume-valid boundary.
  If the shipout committed any effects and dropped the prefix that contained
  that boundary's `World` snapshot position, the hash-only checkpoint marks
  the fallback unavailable for direct rollback. This is the current
  conservative contract: editor/incremental worlds may later choose a
  stronger retention or delayed-materialization policy, but until then callers
  must not interpret an unavailable fallback as resume-valid.
- **Convergence detection**: after re-executing from an edit, compare
  `state_hash` at each checkpoint with the prior run's hash at the same
  input position; on match, splice the old suffix and stop. `state_hash`
  covers: cells touched since the previous checkpoint (from the journal
  slice), code-table generations, arena content hashes of the epoch slice,
  effect-log slice, RNG. It must be a pure function of semantic state —
  never of addresses or allocation order.
  The implemented f26.4 hash is maintained as
  `combine(previous_checkpoint_hash, semantic_slice_hash)`. The slice query
  collects journal cells between checkpoint cursors, canonicalizes global and
  local records to the same semantic cell, compares first-old vs final-live
  semantic content, and hashes only cells whose content changed. Meaning
  cells are keyed by resolved control-sequence name rather than `Symbol`
  number; token, glue, macro-definition, node-list, and deferred-write
  handles are followed to the content they name rather than hashing handle
  words. Node-list content is walked with an explicit worklist, so deep box
  trees do not depend on the Rust call stack. The hash also includes code
  table generation counters, nodes appended to the epoch arena since the
  previous cursor, the uncommitted World effect/input/shell-escape slices,
  stream-buffer state (including terminal-input cursor), RNG state, job clock,
  interaction mode, prepared magnification, and font-dependent content by
  following `FontId` handles to the loaded font's semantic identity (font name,
  input path/content hash, checksum, design size, and selected size) rather
  than hashing raw dense ids.

Derived queries (these fall out; do not build separate instrumentation):

- **Write-set** of a region = journal slice between markers.
- **Read-set** = cells whose epoch stamps were observed (the interpreter/JIT
  records observed `(cell, epoch)` pairs when memoizing).
- **Memo effect replay** = replay the journal slice (forward, using new
  values — see Open Questions on redo records) + effect-log slice.
- **Speculation conflict check** = speculated page's read-set ∩ preceding
  page's journal slice.

---

## 10. Rust enforcement architecture

The type system is the write barrier's bodyguard. The rules:

### 10.1 Crate boundary

- All of the above lives in a `tex-state` crate. `#![forbid(unsafe_code)]`
  crate-wide, with the sole exception of one audited `arena` module (and, if
  needed, the mprotect tripwire in a test-only module).
- Every field of every store is private. Downstream crates (interpreter,
  JIT, drivers) interact only through the public API, which **does not
  contain** any unjournaled mutation.

### 10.2 API shape (the whole invariant in four absences)

- No `get_mut`, no `IndexMut`, no `iter_mut`, no method returning `&mut`
  into any store. Meaning words and scalars are `Copy`; content reads return
  `&[T]` only.
- No interior mutability in state types — no `Cell`, `RefCell`, `Mutex`,
  atomics. `&Env` ⇒ *cannot change* must remain a theorem (memoization
  soundness depends on it), and atomics would poison the barrier's cost.
- Journal lives inside `Env` (and its siblings inside `Universe`), so
  mutating cells and mutating history require the same `&mut` — the borrow
  checker makes bypass unrepresentable in safe code.
- Gullet and lexer code use the public `ExpansionState` capability instead of
  broad `&mut Universe`. The capability includes expansion-safe reads,
  immutable content creation, interning, and magnification preparation, but
  omits input-file reads, input-open context construction, and Env/register/
  code-table/group/snapshot/font-assignment setters. Only the top-level
  expansion/dispatch path carries the additional `InputOpenState` authority
  needed to construct `InputOpenContext`; scanner recursion uses the narrow
  `ExpandNext` capability instead of receiving input-open authority directly.
  Driver `\input` hooks receive the separate `InputReadState` capability
  through `InputOpenContext`, which exposes only content-addressed input reads.

### 10.3 Unforgeable handles

`Symbol`, `TokenListId`, `NodeListId`, `FontId`, `GlueId`, `SnapshotId` are
newtypes with private constructors; only their owning store mints them.
Packed operand fields are decoded back into typed ids *inside* `tex-state`;
raw integers never cross the crate boundary. Stored-word decoders that can
mint handles from packed operands are crate-private; downstream raw decode and
test-only constructor escape hatches are compiled only for crate tests or the
explicit `testing` feature. The `shadow` feature is production-like
verification instrumentation and must not expose raw handle minting.

### 10.4 Builder-then-freeze for content

```rust
let mut b = stores.token_list_builder(); // owned scratch buffer, unfinished, has no id
b.push(tok);
let id = stores.finish_token_list(&mut b); // hash-cons; thereafter &[Token] only
```

A `Builder` is not a `TokenListId`; nothing half-built can be stored into
the environment because no API accepts it. Builders are reusable owned scratch
buffers so the gullet can read frozen lists while building new argument lists.
Node lists likewise; promotion is expressed as the *only* signature for storing
into a box register.
Box-register replacement paths that preserve TeX's current visible box level
(`\box`/`\vsplit`-style same-level writes and clears) are still aggregate
`Universe` facades; downstream crates do not infer or mutate raw environment
ownership directly.

Raw content substores are implementation details of `Universe`: downstream
crates cannot construct `TokenStore`, `GlueStore`, `NodeArena`, or
`SurvivorArena`, cannot call their raw `intern`/append/read APIs, and cannot
freeze a builder by passing `&mut` to a raw substore. Public content creation
and reading instead go through `Universe::intern_token_list`,
`Universe::finish_token_list`, `Universe::intern_glue`, `Universe::freeze_node_list`,
`Universe::finish_node_list`, `Universe::tokens`, `Universe::glue`, and
`Universe::nodes`, which keep handle liveness and rollback watermarks on the
aggregate timeline. Public modules may still expose immutable value types,
handles, and the builder types returned by `Universe`; their constructors and
raw store-finish hooks are crate-private unless compiled for crate-local tests.
Loaded fonts follow the same aggregate rule: `Universe::intern_font`,
`Universe::font`, `Universe::font_name`, and the font parameter/current-font
facades are the public boundary. The three-by-sixteen TeX math-family font
selectors (`\textfont`, `\scriptfont`, `\scriptscriptfont`) are Env-side
font cells beside the current-font selector, so local/global assignment,
group exit, and snapshot rollback use the same barriered journal path as
other TeX state. Font store rollback is watermark based like the interner and
other immutable content stores; rolling back a `Universe` snapshot truncates
fonts loaded after the snapshot, while ordinary TeX group exit only restores
Env-side meanings/current-font/math-family/fontdimen banks through the
journal and does not unload immutable font objects.

### 10.5 Effects as capability

Only `World` owns file handles, RNG, clock. Backed by CI lints:
clippy `disallowed_methods` / `disallowed_types` denying `std::fs`,
`std::io::stdout`, `std::time`, `rand` outside `tex-state::world`.

### 10.6 `Universe` and concurrency

```rust
pub struct Universe { env: Env, tokens: TokenStore, nodes: NodeArenas,
                      world: World, /* scalars */ }   // Send, no shared internals
```

One owned value = one isolated timeline. Speculation = move a rolled-back
clone to another thread. No locks or atomics in the hot loop; `&mut
Universe` *is* the isolation. `rollback(&mut self, &Snapshot)` is a method
on `Universe` only (atomicity rule, §9). The implemented `Universe` wraps a
private `Stores` composition so the M1/M2 liveness and aggregate-rollback
discipline carries forward without exporting a `Stores` checkpoint path.
Because `Symbol` dense ids can be reused after interner rollback, public
meaning writes also live on `Universe`; the facade rejects symbols that are no
longer live in its owned interner before mutating Env cells. `Universe`
snapshots also carry an owning timeline identity and group depth: rollback
rejects snapshots from another timeline and snapshots invalidated by exiting
the group that enclosed their checkpoint. The owning timeline identity is
derived from a private per-`Universe` owner token plus a per-token random
nonce, not from a global counter, lock, or atomic; cloning a `Universe`
allocates a fresh token and nonce and therefore creates a distinct snapshot
timeline. The nonce is part of the snapshot owner identity only; semantic
state hashes never include owner-token addresses or nonces.

### 10.7 The JIT bypass, contained

Compiled code emits raw loads/stores; it cannot call `Env::set`. Containment:

- `tex-state` exports a sealed `layout` module: `#[repr(C)]` guarantees,
  field offsets, and the barrier as a specified instruction sequence
  (identical to what `Env::set` compiles to — verify by inspecting asm).
- The codegen crate is the one privileged consumer, inside the workspace
  trust boundary, `unsafe` confined there.
- The contract is enforced **differentially**: fuzzed programs under
  interpreter and JIT must produce byte-identical journals and state hashes
  (§11). A JIT that skips a barrier fails replay identity immediately.

---

## 11. Verification plan

1. **Replay identity (the defining property, fuzzed).** Snapshot → random
   op sequence → rollback → assert state-hash equality with pre-snapshot
  hash. Any unlogged semantic mutation, including pending `\aftergroup`
  payloads and page-builder contribution/current-page state, survives rollback
  and diverges the hash. Run
   under cargo-fuzz/proptest from day one; this test *is* the invariant.
2. **Shadow mode.** Build flag: every `set` also updates a shadow
   `HashMap<CellId, u64>`; periodic full comparison localizes divergence to
   the offending op. Shadow mode must not enable test-only raw constructors;
   combine it with the explicit `testing` feature only for replay/fuzz tests.
3. **mprotect tripwire.** Paranoid debug build: arenas in `mmap` regions
   held `PROT_READ` except during barrier methods; rogue stores — including
   from `unsafe` arena internals or future FFI (fonts/ICU) — fault at the
   guilty instruction. This is the net under the failure class the type
   system cannot see.
4. **Differential JIT replay** (once codegen exists): identical journals and
   hashes vs. the interpreter on fuzzed programs.
5. **Semantics conformance**: grouping/`\global`/`\aftergroup` behavior
   validated against pdfTeX on targeted micro-suites before anything is
   built on top (the journal replaces the save stack; it must be
   *indistinguishable*).

## 12. Performance budgets (regressions here are bugs)

- Environment read: 1 indexed load; no branches beyond decode.
- Write barrier: ≤ ~5 straight-line instructions; skip path (already
  stamped this epoch) = compare + branch-not-taken.
- Snapshot take: O(1), < 1 µs.
- Journal growth: ≤ tens of KB per typical page (coalesced).
- Rollback: proportional to journal slice + arena truncation; target < 1 ms
  per page of history for typical documents.
- Lexer fast path: no code-table tree access while generations unchanged.
- Zero atomics/locks on the single-threaded hot path.

Benchmarks to stand up early: meaning-lookup microbench, barrier microbench,
snapshot/rollback cycle, and a macro-expansion torture loop (expl3-style
`\csname` churn) to keep the interner honest.

## 13. Milestones

1. **M1 — Environment + journal.** Interner, `Env`, barrier, groups as
   journal markers, `\global` tagging/compaction. Exit: conformance
   micro-suite vs pdfTeX semantics + replay-identity fuzzing green.
2. **M2 — Content.** Token store (builder/freeze, hash-consing), node
   epoch arenas + survivor promotion. Exit: replay identity across content
   watermarks; `\ifx`-as-id-compare correct.
3. **M3 — Snapshots.** `Universe`, snapshot tuple, rollback, code tables
   with CoW pages + generations, `World` virtualization + effect log,
   commit-at-shipout. Exit: rollback/replay fuzzing including effects;
   convergence hashes stable across identical runs.
4. **M4 — Fast paths.** SIMD lexer under generation guards; read-set
   recording; first memoization consumer (box-level). Exit: budgets in §12
   met under benchmark suite.
5. **M5 — Privileged consumers.** JIT layout contract + differential
   replay; speculative page execution using snapshot forks.

Milestone order is deliberate: every guard, version, and stable address the
later features need already exists in the state layer by the time they land.

## 14. Open questions

- **Undo-only vs undo+redo records.** `(cell, old)` suffices for rollback;
  `(cell, old, new)` (one extra word) enables forward replay for memo-effect
  application and jumping forward between retained checkpoints without
  re-execution. Leaning undo+redo; decide at M1 by measuring journal volume.
- **Epoch width.** u32 epochs overflow in pathological sessions; u64 doubles
  the stamp arrays. Likely u32 + session re-stamping at a safe point.
- **Hash-index rewind** for interner/token store: lazy rebuild vs
  insertion-ordered rewindable table. Start lazy; revisit if editor-loop
  profiles show rebuild cost.
- **Glue representation**: resolved in M2 as hash-consed immutable glue specs
  (`GlueId`, mirroring Knuth's ref-counted glue) for snapshot sharing.
- **Marks/inserts/penalties interaction with survivor arena** — spec the
  promotion rules precisely at M2.
- **Lua/scripting state** (if ever): must live behind the same
  snapshot/effect discipline or be excluded by construction. Deferred.
