# Core Engine State

Status: authoritative current contract.

This document specifies the implemented `tex-state` ownership, mutation,
identity, history, effect, and snapshot model. Algorithmic consumers are
described in [architecture.md](architecture.md).

## 1. Goals and non-goals

The state layer provides fast interpreter reads, one enforceable mutation
boundary, exact TeX grouping, rollback, durable checkpoints, semantic hashing,
and immutable content ownership.

The implementation does not reproduce TeX82's memory layout. It preserves TeX
semantics while using typed stores, compact words, opaque handles, and Rust
ownership. It does not permit untracked mutation or host I/O for performance.

## 2. Store overview

`Universe` owns the complete live engine substrate:

| Store                | Contents                                          | History model            |
| -------------------- | ------------------------------------------------- | ------------------------ |
| Interner             | control-sequence names and semantic atoms         | append-only watermark    |
| Environment          | meanings, parameters, registers, current fonts    | journaled writes         |
| Sparse registers     | e-TeX register overflow                           | journaled map/page roots |
| Code tables          | cat/lc/uc/sf/math/del codes                       | copy-on-write pages      |
| Token store          | durable token lists and semantic identities       | frozen + watermark       |
| Provenance           | origins and origin-list spans                     | append-only watermark    |
| Source fragments/map | immutable bytes and current editor layout         | roots + watermarks       |
| Glue store           | canonical immutable glue specs                    | frozen + watermark       |
| Node arenas          | compact node words, sidecars, semantic identities | epoch + survivors        |
| Fonts                | immutable TFM/OpenType selections                 | frozen + watermark       |
| Hyphenation          | patterns, exceptions, language state              | snapshot-owned roots     |
| Page state           | contribution queue, marks, insertions, best break | copy-on-write roots      |
| Journal              | undo entries, group and checkpoint markers        | append-only position     |
| World/effects        | inputs, streams, output, clock, randomness        | snapshot/effect log      |

Only aggregate APIs on `Universe` and its owned `Stores` facade may coordinate
changes across these stores.

## 3. Identity: the interner

`Symbol` is a compact runtime key scoped to one owning interner. It is never a
durable semantic identity. Every live symbol resolves to an immutable name and
a canonical semantic atom independent of allocation order.

Handles validate their owner and generation before use. Foreign, stale, or
rolled-back handles fail rather than aliasing later allocations. Format and
checkpoint identities use canonical atoms and content identities, not raw
runtime keys.

## 4. Meaning: the environment

The environment stores dense meanings, parameters, registers, font selectors,
and epoch stamps. Reads are indexed. Every write records enough information for
TeX grouping, rollback, convergence accounting, and cache invalidation.

Local writes restore at group exit. Global writes survive and supersede older
local restoration. Sparse e-TeX registers obey the same rules as dense cells.
Meaning caches are owned above the environment but invalidate through exact
generation signals returned by aggregate mutation and restoration.

pdfTeX mode reserves typed cells in the integer, dimension, and token-list
banks for its 55 parameter names, including three integer alias pairs. Mode
preparation installs the pinned INITEX defaults; ordinary grouping, rollback,
semantic hashing, and format serialization apply without a PDF-specific side
store.

Box slots additionally retain the group depth that owns their visible value.
Destructive `\box`, `\unhbox`, `\unvbox`, and `\vsplit` updates preserve that
owner depth even when executed inside a nested box-construction group: the
void or remainder value crosses inner boundaries, then the prior value is
restored only when its owning group ends. Journal records therefore carry a
box restore depth independently of whether an ordinary assignment was global;
survivor-reference cleanup uses that same depth so a refiled entry keeps its
box node root live across intervening group exits.

## 5. Meaning, sparse tier: the code tables

Unicode code tables use sparse copy-on-write pages with TeX-compatible default
values. Mutation and restoration occur through `Universe`; consumers receive
guarded lookup views, not raw pages. Page roots and generations participate in
snapshots and semantic state.

Hyphenation patterns and exceptions are also state-owned. Pattern loading,
exception mutation, language selection, snapshotting, and format restore pass
through the aggregate boundary.

## 6. History: the journal and write barrier

The journal is the authoritative ordered record of mutable-cell changes.
Groups and snapshots store journal positions rather than copies of the entire
environment. Restoration replays undo entries while preserving monotonically
safe generations so stale read guards cannot become valid again.

There is one semantic mutation boundary:

- callers identify the logical cell or aggregate operation;
- `Universe` validates ownership and liveness;
- the store records history before mutation;
- the write updates generations and semantic bookkeeping; and
- restoration or commit is performed only through the owning aggregate.

No downstream crate receives `&mut Env`, raw restore hooks, partial checkpoint
mutation, or constructors for opaque handles.

## 7. Content: token, provenance, source, glue, font, and node stores

Immutable content follows builder-then-freeze. Builders are private to the
owning boundary, validate all child handles, compute canonical identity, and
publish only complete values.

Durable token lists are hash-consed and carry one canonical semantic identity.
Control sequences contribute interner semantic atoms, so allocation order does
not affect identity. Execution-transient token flows stay in pooled lexer
buffers and enter the token store only when crossing a durable boundary.

Schema-10 format loading installs names, token lists, macro definitions, glue,
fonts, sparse code-table roots, and hyphenation tries as validated frozen
bases. It attaches fresh runtime identity tags and builds ordinary lookup and
semantic-hash indexes in bulk rather than replaying semantic interning or
assignment APIs. Dense record indices remain the canonical raw ids. Job-created
content and code-table/hyphenation mutations extend those bases and follow the
same lookup, snapshot, generation, and rollback rules as a cold store; no
format byte is mutated.

Schema-10 kind 528 installs validated environment cells directly as an
immutable format base, including references into kind 512's frozen node arena.
The existing banks form the mutable job overlay and retain their ordinary
write barrier, journal, grouping, snapshot, and rollback semantics. Core
names, tokens, macros, glue, fonts, code tables, and hyphenation are neither
duplicated there nor reconstructed through their ordinary mutation APIs.
Environment references are checked against the decoded frozen prefixes before
either the base or its stores are published.

The schema-10 publisher is structurally separate from the test-only legacy DTO
restorer. Operation-level test instrumentation proves that normal loading does
not remap detached graph keys, reseal node semantic identities, or replay raw
environment assignments. Loaded-base mutation remains ordinary groupable and
checkpointed overlay work; rollback exposes the unchanged immutable base.

Provenance is diagnostic metadata and does not affect semantic identity.
Packed origins refer to immutable input records or editor fragments. The
current editor layout maps fragment positions to revision coordinates; deleted
fragments produce typed deletion results.

Glue specs and font selections are immutable content. Font program identity is
derived from validated OpenType data and remains separate from host paths or
transport policy.

Node lists live in compact word arenas with typed sidecars. `NodeListId`
contains owner/generation identity and a span. Epoch nodes are cheap to build
and roll back; survivor promotion preserves immutable content that escapes a
transaction or checkpoint. Each frozen list has a canonical semantic identity
composed from decoded node values and child identities, excluding provenance.

## 8. External effects: the virtualized world

`World` is the sole capability for inputs, output streams, terminal text,
fixed job time, randomness, filesystem-like effects, and resource observation.
Engine crates do not call host filesystem, clock, terminal, or random APIs.
`clippy.toml` enforces the principal forbidden methods.

Input streams retain TeX's semantic open/closed state separately from their
byte cursor. Reading the final physical line leaves a stream open; only a
subsequent read attempt past that line closes it, which is the state observed
by `\ifeof`.

Effects are recorded in execution order and published at explicit commit
boundaries. A failed or rolled-back transaction cannot leak writes, artifact
receipts, DVI plans, or auxiliary output. Virtual compile sessions clone or
fork accepted `World` state so output inspection does not mutate the session.

Native search, browser fetch, caching, authentication, and URL selection are
host policies. The engine reports typed missing resources and accepts validated
responses through the same host-neutral session API. A driver resolver may
supply immutable bytes selected from its own storage, but it must pass them
through the narrow `InputReadState` capability so `World` still allocates the
input record, retains the content backing, and gives pending same-run output
precedence.

## 9. Snapshots, rollback, and commit

### 9.1 Canonical semantic-state contract

A semantic state hash is versioned and allocation-independent. It covers every
future-relevant root, cell, input summary, mode/page root, effect position, and
immutable content identity required by the named boundary. It excludes
diagnostic provenance, host paths, allocation capacities, and derived output
caches.

Hash equality is useful only under the checkpoint schedule and validation
contract that produced it. It is not permission to resume arbitrary Rust
continuations or to equate states with unvirtualized external facts.

### 9.2 Universe snapshot substrate

An internal `Snapshot` captures journal/effect positions, store watermarks,
copy-on-write roots, world state, mode/page summaries, and other future-relevant
scalars. Taking a snapshot is bounded and independent of total live document
size; rollback cost is proportional to changed or newly allocated state.

The strong canonical identity used for optional suffix adoption is derived
later, not captured in every snapshot. Executor sinks request it only for a
schedule-aligned boundary they will compare. Its store projection separates
append-only interned content from mutable state. Names, token lists, macros,
glue, and fonts contribute canonical leaf identities to per-store append-only
prefix trees; watermark growth hashes only new leaves and their prefix roots.
Loaded fonts retain their immutable strong identity at load, while the small
rollback-coupled identifier and expansion projection is composed separately.
The derived prefix caches are shared across related generation forks, validate
allocator ancestry before extending, and fall back to canonical reconstruction
after divergent rollback allocation. They are not semantic state. Environment
cells also maintain a persistent deterministic Merkle treap keyed by canonical
semantic cell identity. At a checkpoint, the existing mutation-journal slice
identifies the distinct dirty cells; only those Merkle paths are replaced, and
the root is retained in the store snapshot so rollback and generation forks
restore it in O(1). A full environment walk seeds the root only when a fresh
store or format image is loaded.

Exact comparison composes that environment root with cached canonical roots
for code tables, hyphenation, magnification/font selection, page-builder
collections and persistent node forests, live input, virtual streams and World
scalars, interaction mode, and the append-only PDF ledger. The page and input
projections reuse immutable-root cache keys; PDF state uses rolling semantic
fingerprints and future allocation cursors. One versioned, domain-separated
checkpoint identity is stored only on compared records. Full mutable-store and
page DTO serialization is not part of exact comparison, so unchanged roots are
O(1) and work at a compared boundary is proportional to roots dirtied since
their cached projections. Detached effects and artifacts remain splice-owned
history and are deliberately excluded.

Snapshots are not public restart points. `tex-exec` alone may publish complete
`EngineCheckpoint`s at `JobStart`, eligible `OuterParagraphEnd`, and outermost
`ShipoutComplete`. A checkpoint owns or pins every root needed for later
validation and restoration.

Commit promotes escaping node/content roots, publishes ordered effects and
artifacts, and releases transaction-local history. Failed validation restores
the prior aggregate state atomically.

## 10. Rust enforcement architecture

### 10.1 Crate boundary

`tex-state` owns mutable state and history. Other crates receive `Universe`,
narrow read traits, immutable values, or opaque handles. `tex-out` receives
only detached validated data.

### 10.2 API shape

The public API intentionally lacks:

- raw access to substores or environment cells;
- unchecked handle constructors or word decoders;
- independent restore methods for pieces of a snapshot; and
- effect publication outside the owning transaction.

Testing-only inspection is feature-gated and must not become a production
shadow API.

### 10.3 Unforgeable handles

All content and state handles are opaque. Ownership and generation checks are
performed at aggregate entry points and while decoding child references.
Serialization validates complete DTO graphs before publishing anything into
live stores.

### 10.4 Builder-then-freeze

Mutable builders cannot escape their owning operation. Freeze validates child
liveness, canonicalizes representation, computes semantic identity, and then
mints the public opaque handle.

### 10.5 Effects as capability

Only `World` and aggregate execution transactions can observe or publish
effects. This is a type boundary as well as a testing convention.

### 10.6 Concurrency

One `Universe` is single-owner mutable state. Parallel work uses separate
universes or immutable detached artifacts; no interior mutability is used to
share a live engine.

### 10.7 Future JIT

A future `tex-jit` may consume a sealed layout surface, but it must use the
same write barriers, generations, effect capabilities, validation, and
deoptimization rules. Until that crate exists, no raw layout API is exposed.

## Verification and performance requirements

- `cargo test --tests` is the hermetic default-native correctness gate.
- `scripts/check.sh` enforces formatting and clippy boundaries.
- Snapshot retention and scaling use `scripts/check-snapshot-budgets.sh`.
- Exact fixture and corpus parity defines semantic compatibility.
- Performance changes use the retained state, execution, and whole-engine
  workloads; historical prototype benchmarks are not permanent gates.
