# Core Engine State — Design Plan

Status: draft for implementation
Scope: the state layer (`tex-state` crate) of a modern TeX engine — storage,
mutation discipline, snapshots, and the enforcement architecture.
Out of scope: expansion/typesetting algorithms, JIT codegen strategy, output
drivers. Those are *consumers* of this layer and are referenced only where
they constrain it.

How to read this document: it is a normative design spec with
implementation status woven in. Passages marked **Status:** (and sentences
phrased "the implemented …" / "in the implementation …") describe what
exists today and change as milestones land; everything else states the
design the implementation must converge to. File-level detail lives in
`crates/tex-state/AGENTS.md`.

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
| Provenance store | diagnostic origin records and packed `OriginId` list spans | append-only, not hash-consed | watermark |
| Source map | logical source regions and shared generated backing identities; World regions retain only `InputRecordId` | append-only through aggregate registration | region/backing watermarks + next-position scalar |
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

- Control-sequence live identities use `SymbolId`, while tokens and Env cells
  retain a compact 30-bit `Symbol(u32)` key. Compact keys come from one
  process-wide monotonic domain: rollback removes their local key-to-slot
  mappings but never returns keys for reuse, and sibling forks inherit old
  mappings while receiving disjoint keys for later names. Each interner keeps
  a dense semantic slot table plus an O(1) compact-key-to-slot map, so hot token
  and Env representations do not widen. The semantic interner key
  is `(ControlSequenceKind, spelling)`, where named control sequences and
  active-character control sequences are disjoint namespaces. Thus active `~`
  and the escaped control symbol `\~` resolve to the same printable spelling
  but have distinct symbols and independent Env cells, mirroring TeX82's
  `active_base+c` and `single_base+c` ranges. Backing: bump-allocated UTF-8
  arena + namespace-aware open-addressing hash index. The packed traced-token representation
  reserves 30 payload bits for control-sequence symbols. `Interner::intern`
  therefore reserves from a nonwrapping `2^30` process-key domain and returns
  `InternerError::TooManySymbols` instead of creating an unrepresentable or
  revived key.
- Semantic slots are append-only between rollbacks. Rollback truncates the
  arena to its watermark and removes discarded compact-key mappings, while the
  process key frontier remains monotonic. The hash index either records insertion order (rewindable) or
  is rebuilt lazily on first intern after a rollback — interning after
  rollback is rare, so lazy rebuild is acceptable v1.
- Dense ids make every downstream lookup an array index; stable ids are what
  compiled code embeds.
- `\csname`-manufactured names enter the named namespace through the same
  interner (expl3 does this constantly; the interner must be fast and its
  growth must be watermarked like everything else). Active-character lookup
  has a separate aggregate facade so callers cannot recreate the old spelling
  collision accidentally.

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
- **Loaded-font identity**: the semantic identity of a loaded font is its TeX
  selector/name, selected size, content hash, checksum, design size, immutable
  metrics/parameters, and registered control-sequence identifier. The host
  path used to locate those bytes is `World` input provenance: it remains
  available for diagnostics and exact source reopen during the live session,
  but is excluded from font interning, semantic hashes, and format images. A
  format restore reconstructs semantic font content without claiming that the
  original host path is meaningful or still available.
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
  `Universe` boundary and contains two frozen token-list ids: parameter text
  and replacement text. Diagnostic definition provenance lives in a side table
  keyed by `MacroDefinitionId`; it records the definition origin plus
  parameter/replacement `OriginListId`s and is not part of `MacroMeaning`.
  Downstream code may decode macro
  meanings through public aggregate facades into the `MacroMeaning` aggregate,
  but it cannot mint live macro-definition ids or inspect the raw store.
  Identical token lists are hash-consed by the token store, so separately
  scanned identical replacement bodies receive the same `TokenListId` even
  when their origin lists differ. Separately scanned macro definitions receive
  distinct `MacroDefinitionId`s so the side table can retain their different
  provenance. Macro-definition semantic hashing and `\ifx` comparison use
  flags plus semantic parameter/replacement token-list content, not
  definition-id or origin-list identity.
- Addresses of materialized meaning cells are stable for the process lifetime:
  a sparse vector of optional 65,536-cell segments grows to cover compact
  symbol keys, while only touched segments allocate and segment boxes never
  move.

### Register overflow (e-TeX sparse tier)

Registers 256..32767 per class: two-level radix pages (page = 256 slots),
default-page shared, cloned on first write, entries barriered like dense
registers. This unifies what e-TeX bolted on as separate save-stack
machinery: **one write path for all registers**.

## 5. Meaning, sparse tier: the code tables

Six tables over ~1.1M codepoints; overwhelmingly default-valued; writes are
rare and bursty (verbatim, `\makeatletter`, babel shorthands).

- Representation: a two-level persistent radix over logical 256-entry pages.
  The 4,352-page Unicode domain is covered by 17 fixed root slots, each of
  which optionally points to a 256-way page chunk. Missing chunks and pages
  are the canonical virtual-default representation; INITEX defaults are
  computed directly, so a fresh table allocates no Unicode pages.
- **Copy-on-write at page granularity**: first write to a shared page clones
  it. A detached write copies at most 17 root handles, 256 page handles, and
  one 256-value page, independent of the Unicode domain. A page restored
  entirely to defaults is removed, and a root with no materialized pages
  reuses its table's canonical empty root. Old snapshots keep the old root; no
  journaling of individual entries is needed (the root swap is the undo record
  — the journal stores old roots).
- Each table carries a **generation counter**, bumped on any write. The
  lexer's SIMD fast path is compiled/validated against a generation vector
  and never touches the tree until a generation bump forces reclassification.
  This is the storage-level grounding of catcode speculation. Generations
  represent assignment activity, not effective value changes: a same-value
  code-table assignment still bumps the table generation, though it need not
  copy a page because the table content is unchanged.
- **Status:** in the implemented `tex-state` API, code tables live behind `Universe`:
  reads and writes go through `Universe::{catcode,set_catcode,...}` and
  `Universe::code_table_generations`. `Universe::snapshot` captures the
  structurally shared root pointers and generation counters, and
  `Universe::rollback` restores them atomically with the Env/content tuple.
  TeX groups save the same six roots in a structurally shared group-root
  vector. Local code-table assignments restore those roots at group exit.
  Global assignments update the live root once and append to an immutable,
  structurally shared write history; group exit replays only the suffix since
  that group's entry onto its saved roots. Thus assignment cost is independent
  of group depth instead of eagerly rewriting every open frame. The group-root
  vector and global-write history are both part of the code-table snapshot, so
  checkpoints taken inside groups retain complete rollback state without a
  second save stack outside `Stores`. Restoring a changed root at group exit
  also advances that table's generation so lexer classification cannot outlive
  the restored catcodes.
  Each implemented table root starts at one canonical shared empty root; the
  virtual missing-page sentinel canonically represents every default page.
  The first effective write materializes only its radix path and touched page.
- INITEX defaults are TeX82-compatible for the classic 0..255 range and
  extended over Unicode by the same default rules: ASCII letters have letter
  catcode and case mappings, uppercase ASCII has `sfcode` 999, ASCII digits
  start with variable-class family-0 mathcodes, and ASCII letters start with
  variable-class family-1 mathcodes. Other scalar values keep the normal TeX
  defaults (`other` catcode, zero case codes, `sfcode` 1000, mathcode equal to
  the scalar value, delcode -1) until set.
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
struct UndoRec { cell: CellId, old: u64, new: u64 } // 24 bytes; CellId spans all banks
enum Marker { Group, Checkpoint(SnapshotId) }
```

`CellId` is a 36-bit key stored in a `u64`: 5 bank bits, one global-assignment
bit, and a 30-bit index matching the complete compact `Symbol` domain. Moving
the cell key from `u32` to `u64` does not enlarge `UndoRec`: its two `u64`
value words already gave the record eight-byte alignment and a 24-byte size.
The tagged `Entry` remains 32 bytes. StoreFormat v2 carries the detached raw
cell key as `u64` and validates its reserved bank bits before rehydration.

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
- **Groups are typed journal markers.** This *replaces* Knuth's save stack:
  brace, `\begingroup`, and math-shift boundaries all push `Marker::Group`
  with a boundary kind and bump the epoch; group exit first checks that the
  requested closer matches, then walks back to the marker restoring records.
  The same aggregate entry/exit also saves and restores code-table roots and
  releases `\aftergroup` payloads for input-stack replay. Same records, same
  log, one mechanism; math entry does not maintain a parallel save stack.
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
  identical lists get identical `TokenListId`s. Macro provenance uses parallel
  `OriginListId` spans and never participates in this token-list identity;
  memo keys are hashes and identical expansions share storage across snapshots
  automatically.
- Backing: bump arena + hash index; rollback = watermark truncation + lazy
  index repair (same policy as interner).

### Provenance store

- Token provenance is mandatory diagnostic side-channel data, not semantic
  token identity. Source-frame pending queues store packed traced token words,
  but input-summary equality and semantic hashing decode them back to `Token`
  so origin differences do not affect snapshot convergence. `OriginId(0)` is
  the reserved Unknown/Bootstrap value and consumes no arena record. Low
  payloads encode live logical `SourcePos` values directly, while high-bit
  values address the origin-record arena; the layout, constructors, accessors,
  and decoder are private. `OriginListId::EMPTY` is the reserved
  empty origin-list span.
- The store owns one append-only `OriginRecord` arena and one append-only
  packed `OriginId` arena addressed by `OriginListId` spans. It is deliberately
  per-instance and not hash-consed: identical origin records or lists may have
  different ids. `OriginListId` is a generation-tagged runtime identity.
  Packed arena `OriginId` values remain 32-bit token-side keys, but those keys
  are process-unique and never reused; a rollback removes their O(1) lookup
  entries while truncating records, so neither rollback nor a sibling fork can
  make a stale packed origin resolve to replacement diagnostic content.
- Origin-list builders are finished through the aggregate `Universe`/`Stores`
  API in parallel with token-list builders. Token-list replay frames carry a
  `TokenListId` plus an `OriginListId`; when the origin span is present its
  length must match the semantic token list length. Stored non-macro token
  lists without an origin-list home replay with synthetic per-replay-kind
  origins in v1 rather than using any global `TokenListId` to `OriginListId`
  map.
- Allocation is infallible from the engine's perspective. Origin-record
  capacity overflow saturates to `OriginId(0)` so diagnostic provenance never
  aborts a TeX compile; origin-list capacity overflow degrades to the empty
  origin-list span. Generated token runs that share one synthesized origin may
  allocate a repeated-origin span directly, avoiding a temporary builder buffer
  while preserving the packed-list representation.
- The public boundary is aggregate-owned. Callers allocate and read source,
  macro-invocation, inserted, synthesized, synthetic/bootstrap, and
  origin-list builder APIs through `Universe`/`Stores`-style APIs; raw
  provenance store mutation and unchecked `OriginListId` construction remain
  internal or test-only. Macro-definition side-table reads are best-effort and
  resolve to unknown/empty provenance when an entry is absent. Provenance
  appends are not journaled and are not part of memo redo slices; execution
  reconstructs them when replaying. `Universe` also exposes read-only
  provenance/source-map lengths, logical bytes, and retained-capacity
  statistics for benchmarks and diagnostics. Direct-delivery measurement uses
  benchmark-only id inspection rather than a production per-token counter
  write; none of these values participate in semantic hashing.
- User-facing provenance is resolved only at diagnostic formatting
  boundaries. Errors own a `DiagnosticSite`: a primary origin, bounded labeled
  related origins, and one parent-linked macro-invocation head captured while
  relevant replay frames are live. No site stores rendered paths, excerpts, line
  indexes, or display widths. `ProvenanceResolver` reads live origin records,
  world input records, and interned names to render source labels, exact
  half-open source ranges, source line/caret context, and the captured macro
  expansion trace. It uses one-based reports, eight-column tab stops, Unicode
  display-cell widths, a one-cell caret for empty ranges, and first/last lines
  for multiline ranges. It treats
  `OriginId(0)`, rolled-back ids, missing origin-list spans, and absent source
  metadata as unknown provenance rather than reporting a secondary error.
- Raw `OriginId`s are valid only while their append-only provenance records
  remain live. Any diagnostic that must survive speculative/replayed execution
  rollback must be rendered to text before rolling back past its provenance
  watermark. Expansion backtraces follow persistent parent-linked invocation
  records from the head captured when an error crosses the lexer, expansion,
  or execution boundary; they are not reconstructed from mutable
  current-location state or stored beside individual tokens.
  Coordinate rendering stays lazy so future splice-time line-delta remapping
  can be inserted at the resolver boundary.
- Token-list delivery treats a missing or rolled-back origin-list span as
  unknown diagnostic provenance while continuing to deliver the still-live
  semantic token list. Macro invocation records link to their parent, and the
  input stack maintains the active chain head in O(1). The innermost head
  popped during one delivery attempt is retained until the next attempt, so
  traces neither disappear at an EOF/pre-token error nor leak onto unrelated
  later tokens. No body-token wrapper record is allocated.
- Provenance statistics distinguish live records, origin-list spans/entries,
  source regions, generated backings, and logical bytes from retained arena
  capacity. Aggregate rollback restores every live counter to its snapshot
  value; backing vectors may retain capacity for reuse, which is diagnostic
  memory rather than live timeline state.

### Source map

- `tex-state` owns one append-only logical `u64` source-position space. Each
  registered source receives a disjoint byte range plus one EOF/empty anchor;
  `SourcePos` construction stays private and `SourceSpan` construction checks
  both half-open endpoints against the region selected by the low endpoint.
- Registration is an aggregate `Universe`/`ExpansionState` operation. World
  regions retain only a validated live `InputRecordId` and byte length, so
  `World` remains authoritative for content bytes. Generated and memory
  regions retain a shared immutable `Arc<[u8]>` under an explicit backing id.
  Raw map/backing mutation is not exposed downstream. Successful input
  registration also returns an opaque `RegisteredSource` capability. A live
  source frame uses it to encode in-range direct positions without repeating
  a source-map lookup; the capability exposes neither raw positions nor raw
  origin encodings, and wide fallback still goes through aggregate validation.
- Store snapshots add region/backing lengths, the logical-position state, and
  an O(1) region-identity mark. Aggregate rollback truncates these with
  provenance while `World` restores its input-record identity watermark.
  `InputRecordId` is a generation-tagged live capability: rollback advances
  the non-restored generation before a discarded record slot can be reused, so
  stale or foreign records cannot resolve to replacement content. Logical
  `SourcePos` ranges are never reused in ordinary allocation, including across
  sibling forks, and region identity validation prevents reused `SourceId` or
  backing slots from making stale direct origins observe discarded content.
  Resolver line starts are derived lazily from stable immutable backing; no
  mutable cache keyed by reusable ids is retained. Source-map identity and
  diagnostic bytes are excluded from semantic hashing.
- Ordinary one-scalar backed source tokens encode their starting `SourcePos`
  directly in `OriginId`, so the dominant lexer path performs no provenance
  append. The direct payload is only an encoding capacity: regions continue in
  logical `u64` space and validated `SourceSpan` arena records provide the
  fallback above it, degrading to unknown only if that arena is also full.
  Phase 6 adopts this layout after controlled measurements: ASCII and mixed
  UTF-8 logical source/provenance bytes fall by 95.73% and 93.93%, with median
  throughput improving 5.43% and 2.56%; no primary workload regresses more
  than 5%. Flat `Source` records remain a degraded compatibility form only for
  explicitly unregistered legacy/test origins and are not emitted by traced
  production inputs.

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
- Every frozen epoch list receives a generation-tagged allocation identity
  from the common runtime-identity substrate (§10.3). `NodeArena` resolves its
  dense allocation slot to the compact word span in O(1). Identity and storage
  watermarks roll back atomically; the generation does not rewind, so equal or
  covering word-span reuse cannot revive a discarded `NodeListId`. Survivor
  handles retain their root-relative packed representation and ownership rules.
- The epoch arena is one compact eight-byte `NodeWord` stream plus aggregate
  per-kind sidecars. Immutable packed `NodeListId` spans are minted only by
  `NodeListBuilder::finish(&mut NodeArena)`. Builders are owned scratch
  buffers; finishing encodes and clears them. Consumers traverse `NodeList`,
  `NodeIter`, and `NodeRef` logical views; neither epoch nor survivor storage
  retains a decoded `Node` mirror, and raw words/sidecars never cross the state
  boundary. Child lists inside newly-frozen
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
- Loaded immutable fonts carry a derived 256-entry width projection. Compact
  list scans expose an opaque same-font byte-character iterator, validate the
  font once per run, and accumulate widths in exact source order; no raw word,
  mutable cache, rollback field, or hash-excluded semantic state crosses the
  aggregate boundary. Phase 6 measurements show 20–42% faster hpack kernels,
  6–9% faster paragraph workloads, and about 45% lower peak process memory,
  with no end-to-end regression above 5%.
- Feature-gated node measurement computes logical and allocator-retained bytes
  on demand from the actual epoch/live-survivor/recycled vectors, including
  the epoch generation-tag and span tables, and records process-local
  promotion scratch/timing outside `Universe`. The largest canonical storage
  is one coherent observation ordered by logical bytes and then retained
  bytes; epoch candidates are recorded after span minting, and totals are sums
  of the complete column record, including owned whatsit payloads. It is absent
  from normal builds and never becomes snapshot, rollback, hash, or replay
  state; the full reproducible accounting and adoption evidence live in
  `node_word_arena.md`.

## 8. External effects: the virtualized world

Nothing in the engine touches the OS directly. A single `World` object owns:

- **Content-addressed inputs**: `World::read_file` and `\openin` reads
  return bytes plus a stable `ContentHash`, and append an `InputRecord` to
  the snapshot-owned World state. Its `InputRecordId` is a two-word runtime
  capability validated by `World`; snapshots retain an O(1) identity
  watermark and rollback never revives a discarded record when its dense slot
  is reused. Cloned timelines accept inherited records but reject each other's
  later reads. The capability has no serialized representation; format loading
  starts a fresh World/session identity timeline. The real backend is the only
  engine code that uses host files;
  the in-memory backend exposes the same API for hermetic tests and corpus
  drivers. `\read` terminal input is also owned by `World`; in-memory worlds
  keep a replayable terminal line buffer plus a snapshot-owned cursor, while
  real worlds read stdin only through this boundary.
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
- **Handle-bearing effects** enter the log through aggregate admission.
  In particular, `Universe::record_deferred_write` validates the unexpanded
  `TokenListId` against its owning live token-store timeline before `World`
  appends the record; `World` cannot publicly persist this Stores capability.
  Stale, foreign, or reused-slot handles therefore fail before the effect
  prefix changes.
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
  that job-start clock into `\time`, `\day`, `\month`, and `\year`. Real
  worlds honor `SOURCE_DATE_EPOCH` before consulting the host clock so parity
  regeneration can pin Umber and the reference TeX to one timestamp.
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

### 9.1 Canonical semantic-state contract

Checkpointing, convergence hashing, format images, restoration validation,
and dependency recording use one field inventory.  A field belongs to exactly
one of these buckets:

1. **TeX-semantic state** can change future tokens, diagnostics, nodes,
   effects, or committed artifacts.  This includes all live `Stores` roots and
   selectors, code-table and hyphenation content, interaction and prepared
   magnification state, page-builder state, virtualized `World` state, and the
   semantic portions of the input, expansion, and mode nests.
2. **Resume-critical continuation state** describes where the implementation
   is inside that computation.  It includes source and token-list cursors,
   macro arguments, conditional/alignment/scanner phases, mode-list roots,
   pending horizontal characters, and the effect boundary from which replay
   remains possible.  It is hashed whenever a difference can change future
   behavior, even when it is not itself a TeX data structure.
3. **Derived acceleration and diagnostic state** can be discarded and rebuilt
   from buckets 1 and 2 without an observable change.  Hash scratch buffers,
   lookup indexes, memo tables, decoded-node caches, allocation history,
   runtime namespaces, source origins, registered-source capabilities, host
   paths, and diagnostic provenance belong here.  They are neither hashed nor
   serialized as semantic format content.  Runtime capabilities needed for
   liveness validation may still be retained by an in-memory snapshot, but do
   not participate in semantic equality.

The owning boundary for buckets 1 and 2 is an **`EngineCheckpoint`**, not a
larger `Universe`.  It atomically composes an opaque `UniverseSnapshot` with
rooted input/gullet state, `ModeNest` state, any explicit scanner/alignment
continuation, and effect-boundary metadata.  `Universe` remains the sole
mutation, liveness-validation, and store/world rollback authority; the engine
coordinator is responsible for synchronizing pipeline-owned roots immediately
before capture and for restoring all components together.  No component may
be restored or published after another component fails validation.

An `EngineCheckpoint` has an explicit schema version.  In-memory schema
changes are source compatibility changes for checkpoint consumers.  Durable
formats use their own versioned, handle-free DTO and may contain only a
validated quiescent subset: complete TeX-semantic format state, empty input
and page/mode continuations, no pending effects, and no Rust-stack scanner
continuation.  Format compatibility is therefore explicit conversion between
versions, never best-effort decoding of a newer graph.

`ResumeValid` means every bucket-2 continuation is either absent at a declared
quiescent boundary or represented by a validated root in the checkpoint.
`HashOnly` is an observation of buckets 1 and 2 for convergence; it is never a
restart capability and carries only a fallback to a prior retained
`ResumeValid` boundary.  Hashing includes every bucket-1 field and every
behaviorally relevant bucket-2 field, follows handles to semantic content,
and excludes bucket 3.  Thus equal hashes assert equal future behavior under
the documented checkpoint schedule, independently of allocation order,
origin recording, or host resource location.

Derived-state review tests must prove the exclusion rule by clearing or
rebuilding each cache/index and observing identical semantic hashes and
output.  Completeness tests vary each bucket-1/2 field independently and prove
either a different hash or exact rollback/replay, including nested input,
macro/conditional, alignment, math, mode, and output state.

### 9.2 Universe snapshot substrate

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
  state required after a source is reopened: original physical line start,
  content-end and terminator ranges, the current normalized UTF-8 line and its
  canonical in-line byte cursor, scalar column, lexer N/M/S state, queued
  traced synthetic tokens such as a blank-line `\par`, token-list replay
  positions and origin-list ids, macro-body replay argument slots, open
  condition frames, the last popped source frame, the next-source-id allocator
  high-water mark, and the Unicode `^^` configuration. Synthetic
  `\endlinechar` positions retain a zero-width anchor after the physically
  backed retained prefix. Condition frames are snapshot-owned input frames; each
  carries its conditional family (`\if...` or `\ifcase`), current limb
  (`\if`, `\or`, or `\else`), whether condition operands are still being
  evaluated, current/previous taken bits, `\ifcase` `\or` count, and skip
  nesting depth needed to resume token-level skipping.
  Before publication, `Universe::set_input_summary` walks the complete graph
  and validates every source registration and World record, compact symbol,
  token/origin-list pair (including all macro arguments), and provenance
  origin. Validation finishes before mutation, so a stale, foreign, or
  post-rollback-reused capability cannot publish even a valid prefix. Lexer
  publication summaries register dormant source frames first. Registered
  source capabilities remain runtime-only identity and are excluded from
  semantic equality and hashing.
  Durable source reopen identity is not allocated by `tex-lex`; each frame
  carries a generation-tagged `World` input-record capability. The `World`
  snapshot retains its identity watermark and pins file/editor content by
  content hash, then validates the record and recreates the `InputSource`
  before these frame summaries are applied.
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
  boundary. `World` also retains the ordered artifact ids as append-only
  downstream notifications so driver composition cannot lose a shipout that
  occurs inside a nested box or alignment scanner. This sequence is committed
  output history: like the artifacts themselves it is neither rollback state
  nor part of semantic convergence hashing.
  The flushed effect prefix is dropped too, leaving only the
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
  effect-log slice, RNG. Each slice hash must be a pure function of semantic
  state — never of addresses or allocation order.
  **Status:** the implemented f26.4 hash is maintained as
  `combine(previous_checkpoint_hash, semantic_slice_hash)`, a fold over the
  checkpoint timeline. The checkpoint `state_hash` is therefore
  **checkpoint-schedule-relative**, not a canonical fingerprint of the reached
  semantic state: two runs produce equal hashes at a boundary only when they
  applied the same semantic changes *and* took checkpoints at the same
  positions under the same policy (hash-only checkpoints advance the fold
  too). Incremental re-execution satisfies this by construction — the driver
  replays the same checkpoint policy over the same suffix — and comparing
  hashes across different checkpoint partitions can only produce false
  non-convergence (a wasted re-execution), never a false match beyond
  ordinary 64-bit collision odds. Decided (umber2-dur.15, tentative): the
  hash stays schedule-relative; if a consumer ever needs a
  schedule-independent semantic fingerprint, that is a new API, not a
  reinterpretation of this one. The slice query
  collects journal cells between checkpoint cursors, canonicalizes global and
  local records to the same semantic cell, and hashes only cells whose content
  changed. Each touched cell's final-live semantic content is reduced to one
  deterministic fingerprint and cached as the next checkpoint's baseline;
  the first checkpoint after construction or rollback derives a missing
  baseline from the journal's first-old word. This avoids repeatedly walking
  old and final content trees while keeping snapshots O(1): the cache is
  derived, excluded from `Snapshot`, copied with a fork, and cleared on
  rollback before being rebuilt lazily. Meaning
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
  than hashing raw dense ids. Diagnostic provenance records and origin-list
  arenas are explicitly excluded from semantic hashes.

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

`SymbolId`, `TokenListId`, `MacroDefinitionId`, `NodeListId`, `FontId`,
`GlueId`, `OriginListId`, `OriginId`, `SourcePos`, and `InputRecordId` are
opaque values with private production constructors; only their owning store
mints them.
Packed operand fields are decoded back into typed ids *inside* `tex-state`;
raw integers never cross the crate boundary. Stored-word decoders that can
mint handles from packed operands are crate-private; downstream raw decode and
test-only constructor escape hatches are compiled only for crate tests or the
explicit `testing` feature. The `shadow` feature is production-like
verification instrumentation and must not expose raw handle minting.

Timeline-owned handles additionally obey one common allocation invariant.
A live runtime identity is the compact tuple `(namespace, generation, slot)`:
the namespace is a fresh private nonce for an allocation branch, generation is
a checked nonzero counter, and slot is the store's dense index. Each
rollback-truncated store retains a parallel allocation tag per live slot.
Validation is therefore O(1): bounds-check the slot and compare its recorded
`(namespace, generation)` tag with the handle. It requires no hash table,
unsafe code, interior mutability, or scan of rollback history.

Rollback truncates the tag table with semantic storage, but the active
generation is timeline metadata and is never restored from a snapshot. Before
any discarded slot can be reused, rollback advances the generation. Generation
overflow is an explicit exhaustion error and leaves rollback unchanged; it
must start a fresh aggregate timeline rather than wrap. A fork copies inherited
slot tags, so inherited handles remain valid in both descendants, then selects
a fresh namespace for new allocations so sibling handles are foreign. Snapshot
marks retain the tag at their live frontier; a mark from a branch already
discarded by an older rollback is rejected instead of silently applying to a
same-length replacement suffix. Immutable canonical prefix entries may use the
reserved built-in namespace only when their meaning is identical in every
store (for example, the empty token list); they are values, not foreign live
capabilities.

Live handles implement neither `serde::Serialize` nor `serde::Deserialize`,
and handle-bearing `Node` and math aggregates deliberately expose no blanket
serde path either. Format images, committed page artifacts, memo records, and
future whole-engine checkpoint files use versioned DTO-local dense references
or semantic content hashes. In particular, format node graphs use private
`FormatNode`/`FormatListKey` records, including a typed box-register value
instead of disguising a wire key as a `NodeListId`. Loading validates and
remaps those DTO graphs before minting fresh runtime identities through the
aggregate `Stores`/`Universe` restore boundary. Aggregate in-memory
snapshots instead include each store's O(1) identity watermark alongside its
content watermark and restore both atomically. `tex-state::identity` implements
this substrate; migration of existing token/glue/font/macro/provenance/source
and node handle layouts is tracked separately so individual stores cannot
invent incompatible generation schemes.

The complete production handle matrix is:

| Handle | Owner | Live runtime identity | Compact stored form | Durable DTO | Snapshot mark | Rollback and fork rule | Validation API |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SymbolId` | `Interner` | generation identity plus its compact key | process-unique, nonreused `Symbol(u32)` in tokens and Env cells | DTO-local `FormatName` table index; fresh live id and compact key on load | `InternerMark` spans/bytes + identity mark | discarded slots retag and their key mappings are removed; compact keys never return to the process frontier; forks inherit old keys and allocate disjoint new keys | O(1) key-to-slot `Interner::resolve_stored` plus `contains_id`, exposed through `Stores`/`Universe` reads and writes |
| `TokenListId` | `TokenStore` | 16-byte generation identity; universal empty builtin | dense `u32` in Env and macros; private format DTOs carry table keys | `StoreFormat.token_lists` index; validated keys are remapped to fresh ids by `Stores` | `TokenStoreMark` spans/tokens + identity mark | discarded slots retag; inherited ids remain valid in forks, new ids are branch-local | `TokenStore::contains`/`resolve_stored`, then aggregate token/register/node ingress |
| `MacroDefinitionId` | `MacroStore` | 16-byte generation identity | dense `u32` operand in packed `Meaning` | `FormatMacro` table index | `MacroStoreMark` definition length + identity mark | discarded slots retag; post-fork definitions are foreign to siblings | `MacroStore::contains`/`resolve_stored`, then aggregate meaning access/mutation |
| `GlueId` | `GlueStore` | 16-byte generation identity; universal zero builtin | dense `u32` in Env and node words/sidecars | `FormatGlue` table index | `GlueStoreMark` spec length + identity mark | discarded slots retag; post-fork specs are foreign to siblings | `GlueStore::contains`/`resolve_stored`, then aggregate register/node ingress |
| `FontId` | `FontStore` | 16-byte generation identity; universal null-font builtin | dense `u32` in Env and compact nodes | `FormatFont` table index/content record | `FontStoreMark` lengths/write log + identity mark | discarded slots retag; post-fork fonts are foreign to siblings | `FontStore::contains`/`resolve_stored`, then aggregate font/meaning/node ingress |
| `OriginListId` | `ProvenanceStore` | 16-byte generation identity; universal empty builtin | dense slot is used only in explicit detached/test storage; live replay summaries retain the full handle | no format/artifact DTO; provenance is diagnostic-only | `ProvenanceMark` list spans/entries + list identity mark | discarded slots retag; stale/foreign lists degrade to missing; forks separate new lists | `resolve_stored_list`/`contains_list`, exposed as `origin_list_if_live` |
| arena `OriginId` | `ProvenanceStore` | 32-bit process-unique packed key mapped to a generation-tagged record identity | the same 32-bit key in `TracedTokenWord` and origin lists | no durable DTO; semantic formats/hashes drop provenance | `ProvenanceMark` records + record identity mark; lookup suffix removed | keys are never reassigned; rollback removes lookup entries and forks allocate distinct keys | `contains_origin`/`get`, exposed as `origin_if_live`; failure degrades to unknown |
| `SourcePos`, `SourceSpan`, `RegisteredSource` | `SourceMap` | process-unique `u64` position/range; each region also carries a hidden generation tag | direct origins use the low 31-bit `SourcePos` payload; `GeneratedSourceId(u32)` is reachable only through a validated region | none; source diagnostics are session-local and World bytes use `ContentHash` | `SourceMapMark` region/backing lengths + region identity mark | position ranges are never reassigned; rollback truncates regions; forks allocate disjoint ranges | `region_for_position`/`span`/`position`; `Universe` validates World backing before registration |
| epoch `NodeListId` | `NodeArena` | 16-byte generation identity; universal empty builtin | compact span lives in the arena's O(1) slot table, never in the handle | `FormatNodeList`/`FormatListKey` detached span reference; live handles reject serialization | one `NodeArenaMark` for storage, spans, and identity mark | discarded slots retag atomically with all columns; forks separate new namespaces | `NodeArena::contains`/`get_epoch`, then aggregate node/box ingress |
| survivor `NodeListId` | `SurvivorArena` | packed 64-bit root/start/length inside the reserved survivor namespace; root key is process-unique | the same packed word in box Env cells | `FormatListKey` detached root/span reference; fresh survivor key on promotion | no truncation mark; refcounts are owned by live Env cells and journal records | released keys are never reused even when storage recycles; inherited roots survive clones and sibling forks receive distinct new keys | O(1) root-key-to-local-slot lookup in `SurvivorArena::contains`/`get`, then aggregate node/box ingress |
| `InputRecordId` | `World` | 16-byte generation identity | no compact engine operand; source frames/regions retain the full capability | none; durable identity is `ContentHash` and format loading starts a fresh World | `WorldSnapshot` input length + identity mark | discarded records retag; inherited records survive clones and siblings reject new foreign records | `World::input_record`; `Universe::register_source` also checks byte length |

Several opaque-looking integers are deliberately not full timeline capabilities.
They must not be confused with the matrix above: `Symbol` is a process-unique
nonreused compact stored key that is rehydrated through its owning interner;
it cannot authorize a read after its local mapping is removed. `CellId` uses
the same 30-bit index domain plus bank/global tags in a validated `u64` key;
`SourceId` and
`GeneratedSourceId` are live aggregate-local indexes protected by source-region
identity and non-reused positions; `SurvivorRootId` is only the private packed
component of a validated survivor `NodeListId`; `CellId`, parameter ids, and
`StreamSlot` are fixed value-domain indexes; `EffectPos` is an absolute effect
log cursor; `CheckpointId` and `ConditionFrameToken` are execution labels, not
content handles; and the legacy `SnapshotId` is currently only an internal
journal-marker placeholder. None of these integers authorizes a live-store
read without the aggregate owner and validation path listed above.

Format restore is a validate-then-publish boundary even after the outer image
checksum succeeds. In particular, detached font metrics are revalidated for
their character-table shape and lig/kern, next-larger, and extensible-recipe
references before any font is interned. Lig/kern programs are additionally
limited to the 65,536 entries addressable by their `u16` runtime cursor, and
every continuing step must remain within that cursor domain. The complete
serialized font-bank view is then checked against those DTOs: font and
identifier handles must be live,
every font has at least TeX82's seven parameters, each parameter count covers
the immutable parameter prefix and every stored `fontdimen`, and current,
math-family, and last-loaded font selectors must be live. Only after that
read-only pass may fresh stores be built and raw Env words restored. Thus a
checksum authenticates transport integrity but never substitutes for semantic
validation, and failure cannot expose a partially reconstructed live tuple.
Fontdimen cells use an injective 15-bit-font/15-bit-slot split within the
30-bit `CellId` index: font ids end at 32767 and one-based parameter numbers at
32768. The checked encoder runs before parameter-count or journal mutation;
there is no masking fallback, so 32769 cannot alias fontdimen 1. This changes
no journal layout: `CellId`, `UndoRec`, and `Entry` retain the sizes documented
in §6.

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
Promotion accepts an epoch-owned root whose descendants may already be owned
by either the epoch arena or a survivor root. The survivor arena selects the
source store from each opaque child handle, memoizes each exact source span,
and iteratively copies the mixed DAG into one new survivor allocation. Every
descendant in that allocation is rewritten to the new root, shared spans are
canonicalized once, and no epoch handle crosses the box-register boundary.
This is required by TeX's ordinary ownership flow: `\copy` can place a node
with survivor-owned children on the current page, and `fire_up` then packages
that page into epoch-owned `\box255` material before the register write.
Box-register replacement paths that preserve TeX's current visible box level
(`\box`/`\vsplit`-style same-level writes and clears) are still aggregate
`Universe` facades; downstream crates do not infer or mutate raw environment
ownership directly. A destructive `\box` read recursively copies the visible
survivor DAG into the current epoch before clearing the register. This
Rust-only transfer step is required because a coalesced group-journal write can
release the register's last survivor reference immediately; page and mode lists
therefore never receive an opaque child handle whose former owner has already
been released. Leader box children participate in the same recursive transfer.
Commands that scan an existing box for immediate placement (`\raise`,
`\lower`, `\moveleft`, `\moveright`, and `\shipout`) use the same rule:
shared register material is recursively copied into epoch ownership before the
node leaves the scanner, so later source-register replacement cannot invalidate
page-owned children.

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
the recoverable capacity-aware `Universe::try_intern_font`,
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
on `Universe` only (atomicity rule, §9). **Status:** the implemented
`Universe` wraps a private `Stores` composition so the M1/M2 liveness and
aggregate-rollback discipline carries forward without exporting a `Stores`
checkpoint path.
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
