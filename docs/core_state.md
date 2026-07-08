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
| Environment | meaning word + epoch per symbol; parameters; dense registers | barriered in-place writes | journal |
| Register overflow | e-TeX sparse registers (256..32767) | barriered writes | journal + page roots |
| Code tables | catcode/lccode/uccode/sfcode/mathcode/delcode over Unicode | copy-on-write pages | root pointer + generation |
| Token store | immutable, hash-consed token lists | frozen at birth | watermark |
| Glue store | immutable, hash-consed glue specs (`GlueId`) | frozen at birth | watermark |
| Node arenas | per-epoch bump arenas + survivor arena | frozen at birth; promotion on escape | watermark; refcounts (survivors) |
| Journal | undo records + group/checkpoint markers | append-only | position |
| Effect log | deferred writes, aux/toc/idx, shell escape, PDF objects | append-only, committed at shipout | position + stream buffers |
| Misc scalars | RNG state, interaction mode, current epoch, input-stack summary | barriered / snapshot-owned | copied into snapshot tuple |

A **snapshot is a tuple of positions and roots** into these stores — a few
dozen words, O(1) to take (§7).

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
  register values are `Copy`; no references into the array escape. Unknown
  opcode words decode through a crate-private stored-word codec into an
  opaque raw-meaning value whose fields can be read and re-encoded after
  `Env::get`, but downstream crates cannot decode arbitrary raw words or
  construct arbitrary raw meanings in production builds.
- **Writes**: symbol-keyed meaning writes are exposed through the owning
  `Stores`/`Universe` facade, which validates that the `Symbol` is live in the
  same interner timeline before calling Env's crate-private barriered setter.
  Same for every register bank and parameter table: semantic assignment runs
  the barrier (§6). Journal restore walks use a crate-private
  `Env::restore_raw(CellId, u64)` primitive that bypasses the barrier; it is
  restore-only, not a semantic assignment API.
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
  `Stores`/future `Universe` boundary and contains two frozen token-list ids:
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
- In the implemented `tex-state` API, code tables live behind `Stores`:
  reads and writes go through `Stores::{catcode,set_catcode,...}` and
  `Stores::code_table_generations`. `Stores::checkpoint` captures the
  structurally shared root pointers and generation counters, and
  `Stores::rollback` restores them atomically with the Env/content tuple.
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
- Shipped pages serialize into content-addressed artifacts (the memo/extern
  store) and their nodes are released.

## 8. External effects: the virtualized world

Nothing in the engine touches the OS directly. A single `World` object owns:

- **Output streams** (`\openout`/`\write`, aux/toc/idx): writes append to an
  effect log; stream buffer state *including partial lines* is snapshot
  state. TeX's own defer-`\write`-to-shipout semantics is the model —
  extended to every effect.
- **Deferred-write token lists** are expanded at shipout against the state
  *at the commit barrier*; read-set tracking must therefore cover
  shipout-time expansion, not just mainline execution.
- **Shell escape, PDF object stream, log file**: same discipline — buffered,
  committed at shipout, discarded on rollback.
- **RNG state and clock reads**: owned by `World`, journaled/snapshotted.
- **Inputs** (file reads) are content-addressed and recorded, so a snapshot
  pins exactly what it read (needed for cross-run memo sharing).

Effects **materialize only when the producing page commits** (shipout).
Rollback discards the uncommitted suffix of the effect log.

## 9. Snapshots, rollback, commit

```rust
pub struct Snapshot {
    owner: SnapshotOwner,          // rejects cross-Universe/Stores misuse
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
    state_hash: u64,               // for convergence detection
}
```

- **Take**: O(1) — record positions/roots, copy scalars. Frequency: every
  shipout; every paragraph while an editor session is hot. A snapshot belongs
  to the `Universe`/`Stores` instance that created it. In M2, snapshots taken
  inside a TeX group are valid only while that enclosing group is still open;
  leaving the group truncates the journal below the checkpoint position and
  invalidates those snapshots instead of permitting partial rollback.
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
  their old owners; discard effect-log suffix; restore scalars.
  Pending `\aftergroup` payloads are part of the Env rollback tuple: snapshots
  carry an aftergroup length and rollback truncates payloads pushed after the
  snapshot. The epoch counter is never rewound — rollback bumps it past its previous
  maximum (stale high stamps on restored cells would otherwise bypass the
  barrier's journal push, same failure as skipping the group-exit bump).
- **Atomicity rule (hard invariant)**: meaning cells contain content ids, so
  the journal and the arena watermarks restore **as one tuple, never
  independently** — otherwise every box register dangles. Enforce by making
  rollback a single method on the top-level `Universe` (§10.6); no partial
  rollback API exists. In M1, `Stores` is the implemented subset of that
  boundary (`Env`, interner, content stores, survivor roots, and code-table
  roots); `Env` journal positions, journal walks, raw rollback, and raw root
  restoration are crate-private implementation details behind
  `Stores::checkpoint`, `Stores::rollback`, and the liveness-checking
  `Stores` write facades.
- **Commit barrier = shipout**: page artifact serialized, effects flushed,
  snapshots older than the last live editing anchor dropped. History is
  bounded.
- **Convergence detection**: after re-executing from an edit, compare
  `state_hash` at each checkpoint with the prior run's hash at the same
  input position; on match, splice the old suffix and stop. `state_hash`
  covers: cells touched since the previous checkpoint (from the journal
  slice), code-table generations, arena content hashes of the epoch slice,
  effect-log slice, RNG. It must be a pure function of semantic state —
  never of addresses or allocation order.

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

In M2, raw content substores are implementation details of `Stores`: downstream
crates cannot construct `TokenStore`, `GlueStore`, `NodeArena`, or
`SurvivorArena`, cannot call their raw `intern`/append/read APIs, and cannot
freeze a builder by passing `&mut` to a raw substore. Public content creation
and reading instead go through `Stores::intern_token_list`,
`Stores::finish_token_list`, `Stores::intern_glue`, `Stores::freeze_node_list`,
`Stores::finish_node_list`, `Stores::tokens`, `Stores::glue`, and
`Stores::nodes`, which keep handle liveness and rollback watermarks on the
aggregate timeline. Public modules may still expose immutable value types,
handles, and the builder types returned by `Stores`; their constructors and
raw store-finish hooks are crate-private unless compiled for crate-local tests.

### 10.5 Effects as capability

Only `World` owns file handles, RNG, clock. Backed by CI lints:
clippy `disallowed_methods` / `disallowed_types` denying `std::fs`,
`std::io::stdout`, `std::time`, `rand` outside the effects module.

### 10.6 `Universe` and concurrency

```rust
pub struct Universe { env: Env, tokens: TokenStore, nodes: NodeArenas,
                      world: World, /* scalars */ }   // Send, no shared internals
```

One owned value = one isolated timeline. Speculation = move a rolled-back
clone to another thread. No locks or atomics in the hot loop; `&mut
Universe` *is* the isolation. `rollback(&mut self, &Snapshot)` is a method
on `Universe` only (atomicity rule, §9). Until the full `Universe` exists,
`tex-state::stores::Stores` provides the same public checkpoint/rollback
boundary for the M1 store tuple. Because `Symbol` dense ids can be reused after
interner rollback, public meaning writes also live on `Stores`; the facade
rejects symbols that are no longer live in its owned interner before mutating
Env cells. M2 `Stores` snapshots also carry an owning store identity and group
depth: rollback rejects snapshots from another store timeline and snapshots
invalidated by exiting the group that enclosed their checkpoint. The owning
store identity is derived from a private per-`Stores` owner token, not from a
global counter, lock, or atomic; cloning a store allocates a fresh token and
therefore creates a distinct snapshot timeline.

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
   payloads, survives rollback and diverges the hash. Run
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
