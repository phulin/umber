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

One caller-owned session `ReachabilityStore` physically owns two inline
retained-generation slots. Incremental and composed host sessions borrow it,
and their lifetime prevents every session/generation lease from outliving the
store. Each slot contains one `Universe` plus its dense state, journals, save
stacks, checkpoints, continuations, and generation-typed executor sidecars.
`RetainedStateGeneration` and `RetainedEngineGeneration` are move-only slot
leases, not self-contained arena owners. Inside an admitted slot, `Universe`
coordinates the following live engine stores:

| Store                | Contents                                            | History model            |
| -------------------- | --------------------------------------------------- | ------------------------ |
| Interner             | control-sequence names and semantic atoms           | append-only watermark    |
| String pool          | TeX/Web2C allocation coordinates and recycled names | append-only position     |
| Environment          | meanings, parameters, registers, current fonts      | journaled writes         |
| Sparse registers     | e-TeX register overflow                             | journaled map/page roots |
| Code tables          | cat/lc/uc/sf/math/del codes                         | copy-on-write pages      |
| Durable value arenas | token lists, definitions, glue, and provenance      | external store slot      |
| Provenance           | origins, frames, and source ranges                  | typed arena coordinates  |
| Source fragments/map | immutable bytes and current editor layout           | roots + watermarks       |
| Glue store           | canonical immutable glue specs                      | frozen + watermark       |
| Node payloads        | scratch, mode/page, and durable list arenas         | typed arena coordinates  |
| Fonts                | immutable TFM/OpenType selections                   | frozen + watermark       |
| Hyphenation          | patterns, exceptions, language state                | snapshot-owned roots     |
| Page state           | contribution queue, marks, insertions, best break   | copy-on-write roots      |
| Journal              | undo entries, group and checkpoint markers          | append-only position     |
| World/effects        | inputs, streams, output, clock, randomness          | snapshot/effect log      |

Only aggregate APIs on `Universe` and its owned `Stores` facade may coordinate
changes across these stores.

The slot-local durable arenas are a migration representation. The external
topology is implemented; moving their rows/chunks into store-level storage and
installing safe non-`Copy` roots with direct row release is the immediate next
step. That work may not introduce per-value `Arc`/`Weak`, a registry or search,
unsafe pointers, compaction, relocation, rehome, or another historical slot.

The private HotCore substrate additionally defines the runtime values adopted
by canonical command-input delivery. `TokenWord` is a 4-byte exact
token-only encoding and composes with the existing 4-byte origin into the
unchanged 8-byte traced word. A source coordinate is 4 bytes, a generation-
checked token span is 24 bytes, and a compact input frame is 40 bytes. Frames
and spans carry only copyable chunk identities; the arena candidate or accepted
layer owns each allocation once. `tex-command` reaches the canonical frame
layout through `packed_input`: live source, backup, noexpand, template,
inserted-hook, and ordinary stored-replay levels use the frame as their sole
level identity, and token levels use its 32-bit position as their sole delivery
cursor. Source levels retain the exact 64-bit physical cursor required for
large inputs and diagnostic reconstruction. Detached continuations remain
handle-free and reconstruct fresh frames from their portable identity and
token position. Runtime frames derive no serialization and cannot enter
schema-12 formats. Foreign and stale spans reject at arena admission; admitted
traversal performs no repeated generation lookup. Macro-body and argument
delivery now use admitted 64-record macro chunks, 16-byte argument-span
coordinates, 48-byte activations, and reusable argument/invocation chunks.
The packed macro cache owns no semantic child by itself: the environment root
remains authoritative until one command-level chunk admission retains the live
token and provenance closure. Continuation and schema-12 DTOs remain
handle-free and reconstruct fresh runtime chunks.

An incremental session uses the external store's optional prior slot and one
exclusive current slot. Candidate operations mutate only current; resource
suspension retains the same slot lease and typed continuation across host
turns. Rejection clears current and preserves prior. Acceptance clears the old
prior and changes the current lease's role without moving rows or creating a
third allocation domain. The existing durable arenas remain partitioned by
slot until their bodies migrate into store-level reachability storage.

The same boundary owns TeX82-shaped allocator diagnostics:
`Universe::engine_usage_statistics` combines live usage from the interner,
token, glue, node, font, and hyphenation stores. End-of-job code consumes this
small value projection and never receives raw store access. The aggregate
retains componentwise high-water values at §§125--127 allocation events, so
speculative or checkpointed allocation is still represented at job
termination. Section 283 `unsave` restores or frees existing allocator-owned
values; it does not rescan the complete live closure to rediscover a coordinate
that was already recorded when the value was allocated.

The live-root portion of that diagnostic is a derived projection retained
across ordinary executor operations. Meaning and token-list Env changes update
it at the existing mutation barrier, while the glue watermark is refreshed on
the next observation; transient scanner words and unfinished nodes compose
against it without installing a root. A box-root replacement remains a
conservative lazy rebuild because the projection owns no second node-lifetime
registry. Aggregate rollback applies
its Env restoration receipts while destination owners are still live and
before rejected store suffixes are truncated, so non-box rollback is likewise
O(delta); a restored box root takes the same conservative rebuild path as an
ordinary box assignment. The projection is neither semantic state nor snapshot
payload; its recorded TeX82 high-water coordinates remain authoritative when
it is discarded.

Mutable font parameter banks share a process-configured `font_info` capacity:
TeX82's compiled default is 20,000 words, while the pinned Web2C pdfTeX
configuration selects 8,000,000. The limit is operational and is neither
serialized nor hashed. Only the most recently loaded font can grow, and the
state boundary preflights the complete growth delta against all live banks
before changing its length. Exhaustion therefore leaves the font unchanged
and lets execution issue §580's fatal `font memory` overflow.

The state-owned diagnostic printer also preserves TeX's distinction between
§58 `print_char` and §68 `print_ASCII`. The latter crosses the one-character
string table, so missing control characters use canonical `^^` notation while
remaining subject to the shared diagnostic selector.

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

`CellId` is the canonical identity shared by environment storage, journal
records, exact state hashing, and dependency tracking. Its assignment-scope bit
records whether a journal write was local or global, but that bit is stripped
for semantic identity: both assignments address the same bank/index cell.
Meanings use the `Meaning` bank rather than a separate dependency namespace.
Coarse font dependency projections remain separate because they name semantic
font aggregates rather than individual environment cells.

Every typed environment write returns a canonical mutation receipt containing
that scope-free cell identity and a semantic disposition. `Changed` means the
visible word changed; `Unchanged` means the write barrier still performed any
required TeX save-stack work without changing the word. Restoration walks
deduplicate touched cells and compare their visible values before and after the
complete walk. They additionally report `Retained` when a global assignment
survives group compaction without changing the group-exit value. Box ownership
remains attached to the box write outcome but does not replace this semantic
receipt.

A borrow-scoped count/group episode is an accessor over these same structures,
not a second environment. It writes the fixed count bank through the ordinary
barrier and opens/closes the same typed journal markers as scalar `Universe`
calls. Because the episode admits no observation or checkpoint inside it,
changed-at and exact-identity publication may coalesce until its next group or
completion boundary; the visible value and undo records never do. Active
tracked regions and group/restoration tracing force canonical scalar execution.
An enclosing local-retry snapshot restores the complete aggregate if a later
episode or output barrier fails.

The journal also owns an incremental projection of TeX82 §§273--280's physical
save-stack words for §1334's diagnostic high-water accounting. Appending a
group marker, local save, global supersession, or box save updates that derived
projection with the journal entry; ordinary reads are constant-time and do not
rescan the growing journal. Each entry records the inverse of its projection
mutation, so rollback truncation walks only the removed suffix. This restores
local-save eligibility removed by a later global assignment and keeps the
projection coupled to the journal without replaying retained level-zero
definitions.

Local writes restore at group exit. Global writes survive and supersede older
local restoration. Sparse e-TeX registers obey the same rules as dense cells.
Meaning caches are owned above the environment but invalidate through exact
generation signals returned by aggregate mutation and restoration.

pdfTeX mode reserves typed cells in the integer, dimension, and token-list
banks for its 55 parameter names, including three integer alias pairs. Mode
preparation installs the pinned INITEX defaults; ordinary grouping, rollback,
semantic hashing, and format serialization apply without a PDF-specific side
store.

Box slots live beside the copyable dense metadata as move-only durable
owner-plus-root carriers. Their reversible journal moves owners through group,
operation, and checkpoint lanes; cheap restoration receipts expose only copied
metadata. The durable region owner, not a packed word or raw coordinate, is the
lifetime authority.
Destructive `\box`, `\unhbox`, `\unvbox`, and `\vsplit` updates preserve that
owner depth even when executed inside a nested box-construction group: the
void or remainder value crosses inner boundaries, then the prior value is
restored only when its owning group ends. Journal records therefore carry a box
restore depth independently of whether an ordinary assignment was global; a
refiled record carries its owned closure across intervening group exits.

## 5. Meaning, sparse tier: the code tables

Unicode code tables use sparse copy-on-write pages whose absent entries mean
INITEX's initial values. Mutation and restoration occur through `Universe`;
consumers receive guarded lookup views, not raw pages. Page roots and
generations participate in snapshots and semantic state.

The defaults are exactly tex.web §232 and §240, not the state a format leaves
behind. §232 makes every character `other_char` and then overrides only `^^@`
(ignore), `^^M` (car_ret), space (spacer), `%` (comment), `\` (escape), `^^?`
(invalid_char), and the ASCII letters; `\mathcode`, `\lccode`, `\uccode`, and
`\sfcode` follow the same module, and §240 sets every `\delcode` to `-1`
except the null delimiter period, whose `\delcode` is `0`. The seven
characters left brace, right brace, dollar, ampersand, hash, circumflex, and
underscore are therefore `other_char` in a fresh `Universe`: plain.tex assigns
them (lines 11--17), and Umber -- which has no dumped plain format --
synthesizes that part of the prelude in `umber::prepare_run_stores`, where the
loaded format's state belongs. `Universe::new_with_plain_catcodes` and
`Universe::install_plain_catcodes` expose the same seven assignments to
callers that need a format-loaded engine without executing one.

Hyphenation patterns and exceptions are also state-owned. Pattern loading,
exception mutation, language selection, snapshotting, and format restore pass
through the aggregate boundary.

## 6. History: the journal and write barrier

The journal is the authoritative ordered record of mutable-cell changes.
Groups and snapshots store journal positions rather than copies of the entire
environment. Restoration replays undo entries while preserving monotonically
safe generations so stale read guards cannot become valid again.

Journal positions belong to an explicit baseline. A level-zero snapshot
registers one rollback root on that baseline; cloning or dropping the snapshot
clones or releases that exact root. Snapshots inside an open group rely on the
group lineage and are not retained after that group is consumed. A generation
fork's prefix-retarget authority is also a live restoration root even before
its inherited checkpoint records are rehomed.

After a successful aggregate operation, `Env` makes the current cells the new
baseline and clears the journal only when the consumed operation mark is the
sole level-zero root and no TeX group is open. Clearing drops token, macro,
glue, and box undo owners while preserving the vectors as bounded operation
scratch. Retained checkpoints, open groups, and
fork prefixes keep their exact suffixes. No root registry, environment scan,
or historical-generation compactor participates in this transition.

The rolling state-hash cache folds the first old semantic hash for each
distinct retired cell before its undo owner is released. A journal-baseline
serial prevents the unchanged-cursor fast path from skipping that compact
pending delta. Baseline serials, root counts, capacities, and retirement
timing remain operational and do not enter semantic identity.

There is one semantic mutation boundary:

- callers identify the logical cell or aggregate operation;
- `Universe` validates ownership and liveness;
- the store records history before mutation;
- the environment returns a canonical semantic mutation receipt;
- `Universe` advances exact-cell or font-projection dependency stamps only for
  a `Changed` receipt; and
- restoration or commit is performed only through the owning aggregate.

Equal assignments therefore preserve local/global journal entries,
coalescing, group ownership, and tracing while leaving semantic dependency
stamps unchanged. Group exit consumes final-value receipts after global
compaction; retained and equal restorations do not advance stamps. Rollback
also reports final restoration receipts, while the aggregate dependency
snapshot restore remains responsible for restoring non-environment facts and
monotonic changed-at ancestry. Code-table generations and other coarse
aggregate keys keep their existing mutation paths.

`Universe` also owns the generic tracked-region contract. Beginning returns an
opaque mark containing only aggregate-owned dependency and environment-journal
positions/lineage. It advances the environment write epoch first, so a cell already
written immediately before the region cannot coalesce away the region's first
write. Finishing returns two deterministic detached sequences: observed
dependencies ordered by canonical dependency key, and distinct scope-free
`CellId` writes ordered by cell identity. The journal identifies which cells
were written, but each final value is projected from live state through the
same allocation-independent semantic vocabulary used by dependency
validation; the first-write redo word is not authoritative after repeated
writes.

Journal-compacting operations are a conservative boundary for this first
generic product. A checkpoint, group exit, or rollback after the mark advances
the dedicated journal lineage, so finish rejects the region with a typed
unsupported-timeline result and atomically discards its observations. Group
entry advances the write epoch without changing that lineage, so a region may
enter a group or record assignments inside an already-open group and finish
before that group exits; local and global journal identities collapse to the
same semantic cell.
Nested begin, stale or foreign marks, and unsupported cell projections also
fail closed. Explicit abandon publishes nothing and leaves no active recorder.
One typed poison operation retains the first unsupported-fact reason while a
region is active and is an allocation-free no-op otherwise. Finishing a
poisoned region clears its partial observations and returns the typed reason;
repeated barriers cannot restore eligibility.
Dependency-aware `Universe` getters use an atomic inactive fast path. During
an ordinary tracked main-control operation they project supported execution
environment, font, hyphenation, layout, page, PDF, and virtual `World` reads;
page and PDF aggregate projections share conservative per-family mutation
clocks. Unsupported host facts and irreversible materialization poison the
active region before they influence execution.
The record validates data only: it contains no transition replay authority,
paragraph continuation, mounted output, raw substore, or checkpoint handle.
The exact ordinary-main-control region, exhaustive read/barrier matrix,
lifecycle ownership, and proof obligation are defined by
[tracked_region_coverage.md](tracked_region_coverage.md). That contract does
not grant replay authority or add a paragraph continuation.

No downstream crate receives `&mut Env`, raw restore hooks, partial checkpoint
mutation, or constructors for opaque handles.

This boundary maps to TeX.web §§268--283: `eq_define` (§277) and
`eq_word_define` (§278) save an outer value once per level, global definitions
set level one (§§279--280), and `unsave` (§§281--283) restores a saved value
unless the live entry is global. `\let` uses that same definition path in
§§1219--1221; it has no separate restoration rule. Alignments open one outer
and one per-entry `align_group`, replace the latter after each cell, and unsave
both at completion (TeX.web §§773--800, especially §§774, 791, and 800).
Recovery inserts a closer for the actual current group rather than restoring a
different cell (§1064). pdfTeX.web preserves these rules at §§288--303,
§§1395--1397, §§947--974, and §1240; its extended save-stack records do not
change ordinary equivalent restoration. In Umber, `Env` meaning cells and
journal records implement `eqtb` and the save stack, while `tex-command` owns
alignment template delivery and `tex-exec` owns execution-group entry and exit.
The group-exit boundary also carries the journal walk's ordered old-value
restore/retain records so `Universe` can render TeX82 §283
`\tracingrestores` through §245's shared diagnostic selector.
An alignment-token accounting error must therefore be repaired before group
exit, not by redirecting an `Env` restore to a different control sequence.

## 7. Content: token, provenance, source, glue, font, and node stores

Immutable content follows builder-then-freeze. Builders are private to the
owning boundary, validate all child handles, compute canonical identity, and
publish only complete values.

Durable token lists, macro definitions, glue, and exact origin lists are exact
immutable rows in append-only arenas inside one of the external
`ReachabilityStore`'s two physical slots. Control sequences contribute
interner semantic atoms, so allocation order does not affect identity. Private
copy-only generation coordinates currently select rows by direct indexing;
there is no runtime value-region registry, root-set lookup, content search, or
ordinary-read liveness operation. Token and node semantic identities remain
versioned and domain-separated; their compact hash projections are convergence
evidence rather than an external cryptographic content-verification contract.
Execution-transient token flows stay in pooled lexer buffers and enter durable
storage only when crossing a durable boundary.

An immutable store entry does not become semantic state merely because it was
allocated. A live environment cell, input frame, page root, node edge, PDF
record, or other future-relevant root contributes the referenced value's
canonical identity recursively. Unreferenced token lists, macro definitions,
glue specifications, fonts, node lists, and provenance do not contribute.
Their dense slots, generation tags, capacities, interning tables, and derived
identity caches are physical retention or acceleration metadata. Today an
unreferenced durable row remains until its containing slot is cleared. Direct
non-`Copy` root ownership and store row release are the next implementation
step.

Schema-11 format loading installs names, token lists, macro definitions, glue,
fonts, sparse code-table roots, and hyphenation tries as validated frozen
bases. Loading reserves destination-local arena rows, attaches fresh
runtime identity tags, validates every dense record and reference, and
publishes the complete destination slot only after the image is valid. It does
not build a weak runtime liveness index or replay assignment APIs. Dense record indices remain
the canonical raw ids. Job-created value content uses reusable
generation-safe dynamic slots while code-table/hyphenation mutations extend
their bases; both follow the same snapshot and rollback rules as a cold store,
and no format byte is mutated.

Schema-11 kind 528 installs validated environment cells directly as an
immutable format base, including references into kind 512's frozen node arena.
The existing banks form the mutable job overlay and retain their ordinary
write barrier, journal, grouping, snapshot, and rollback semantics. Core
names, tokens, macros, glue, fonts, code tables, and hyphenation are neither
duplicated there nor reconstructed through their ordinary mutation APIs.
Environment references are checked against the decoded frozen prefixes before
either the base or its stores are published.

The schema-12 publisher is structurally separate from the test-only legacy DTO
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

Node lists live in scratch, mode/page, or generation-durable arenas. Their
copy-only `NodeListId<L>` coordinates contain no lifetime authority. Explicit
promotion walks only declared escaping closures, densely relocates child
coordinates, and changes payload coordinates at the lifetime boundary.

Box cells and undo records contain generation-branded durable coordinates.
PDF form records do likewise. Open modes and the page builder contain
page-lifetime coordinates, while synchronous transforms use operation scratch.
The external store currently owns whole revision slots; page arenas and
operations own their narrower storage. No list or payload has an individual
reference count or root-set entry. The forthcoming durable-row migration adds
move-only roots with explicit store release, not per-value reference counts.

## 8. External effects: the virtualized world

`World` is the sole capability for inputs, output streams, terminal text,
fixed job time, randomness, filesystem-like effects, and resource observation.
Engine crates do not call host filesystem, clock, terminal, or random APIs.
`clippy.toml` lists the principal forbidden methods and
`[workspace.lints.clippy]` denies `disallowed_methods`, so the policy fails any
`cargo clippy` rather than only the `-D warnings` gate. Host-side test
harnesses and tooling that legitimately own filesystem I/O carry a narrow
per-item `#[allow(clippy::disallowed_methods)]` with a reason; engine and test
code that can avoid the read entirely, such as a compile-time `include_str!`,
should do that instead of taking the exception.

Input streams retain TeX's semantic open/closed state separately from their
byte cursor. Reading the final physical line leaves a stream open; only a
subsequent read attempt past that line closes it, which is the state observed
by `\ifeof`.

Effects are recorded in execution order and published at explicit commit
boundaries. A failed or rolled-back transaction cannot leak writes, artifact
receipts, DVI plans, or auxiliary output. Virtual compile sessions clone or
fork accepted `World` state so output inspection does not mutate the session.
Memory-backed retained hosts checkpoint the materialized terminal, log, and
stream prefixes when a resource suspension crosses the host boundary. As the
step replays, an exact suffix/prefix overlap is removed once, preserving both
earlier accepted output and new output after the replay. This makes an eager
commit inside the speculative region idempotent without withdrawing unrelated
materialization. Diagnostic writes, including TeX82 §310 input context, use
that same effect path and therefore survive the accepted memory-backed retry
and native-driver handoff exactly once. Real host publication remains an
irreversible barrier and cannot be rolled back after bytes cross it.
After an effect commit, native downstream files publish as one recoverable
staged set: a failed destination rename restores all pre-publication files and
removes every newly installed member.

Native search, browser fetch, caching, authentication, and URL selection are
host policies. The engine reports typed missing resources and accepts validated
responses through the same host-neutral session API. A driver resolver may
supply immutable bytes selected from its own storage, but it must pass them
through the narrow `InputReadState` capability so `World` still allocates the
input record, retains the content backing, and gives pending same-run output
precedence. Each successful read retains typed origin metadata: immutable
external inputs participate in retained validation and dependency receipts,
while rollback-safe outputs reopened during the same run remain readable and
source-addressable without becoming external cache dependencies.

## 9. Snapshots, rollback, and commit

### 9.1 Canonical semantic-state contract

A live-state identity is versioned and allocation-independent. It covers every
future-relevant root, cell, input summary, mode/page root, virtual stream, and
immutable content identity reachable from the named boundary. Referenced
values contribute canonical content recursively; append-store watermarks,
vector positions, capacities, dead definitions, abandoned provenance,
physical handles, generation tags, cache membership, and revision ids do not.

The identity has no authority over reachability. Reachability is established by
the typed live roots and validated child traversal; neither a probabilistic set
nor a cache hit may make an otherwise dead value semantic. The session-local
identity retains the existing versioned, domain-separated 64-bit aHash
collision contract. Durable files, formats, resources, and artifacts continue
to use their cryptographic `ContentHash` identities.

Live engine identity, revision lineage, and published output are separate
coordinates:

- the optional exact checkpoint projection identifies future live engine state;
- `state_hash` is the schedule-relative rolling lineage token used for replay
  telemetry and checkpoint-schedule validation; and
- detached effect and artifact prefixes identify observable output and are
  compared or spliced in order by their owning output ledger.

No one coordinate substitutes for another. In particular, output cache
membership never enters live-state equality, and matching live state does not
authorize adopting output without the explicit ordered-prefix checks.

Hash equality is useful only under the checkpoint schedule and validation
contract that produced it. It is not permission to resume arbitrary Rust
continuations or to equate states with unvirtualized external facts.

### 9.2 Universe snapshot substrate

An internal `Snapshot` captures journal/effect positions, store watermarks,
copy-on-write roots, world state, mode/page summaries, and other future-relevant
scalars. Taking a snapshot is bounded and independent of total live document
size; rollback cost is proportional to changed or newly allocated state.
Each environment snapshot also carries the O(1) lineage token of its enclosing
group. Rollback may unwind descendant groups entered after capture, while an
exited-and-replaced enclosing group invalidates the snapshot even if the live
stack later returns to the same depth and journal position.

Production command delivery has no private aggregate retry snapshot or
executor-preflight value. Raw or expanded delivery classifies uncommon
resource and late-failure barriers directly from the resident command;
ordinary delivery stores and copies no classification. The PDF family alone
demand-reads live `\pdfoutput` for its DVI-mode retry decision. Typed operand
scanning then completes in place; resource misses retain their prepared
request, and semantic apply commits exact owners directly.
`DirectOperationMark` is a fixed-size, non-restoring cursor over the
operation's environment-journal activity and private immutable-store suffix.
Its command-attempt edge is coordinate-free; `CommandState` owns the sole
ordinary opening mark, while a real resource suspension alone retains a cold
copy for pending-continuation admission. It registers no rollback root, clones no aggregate state, and does not publish,
compute, or advance durable semantic identity. At level zero, a changed direct
operation may retire closed environment-journal history only when no named
checkpoint or fork prefix can restore it. Open groups and retained checkpoints
keep their exact records. Failed private operations discard only their
unpublished allocation suffix; canonical partial-state error paths retain the
semantic scalars TeX specifies. TeX82 §§1030--1038 remain the command-dispatch
authority, and only the named checkpoint schedule is a semantic hashing
boundary.

The string-pool store follows the same bound. TeX82 §44's pool coordinates and
Web2C tex.ch [29.517]'s `search_string` membership are semantic state, but a
snapshot retains only their scalar coordinates and the position in an
append-only unique-string journal. A duplicate search changes neither the
membership index nor the journal. Rollback removes only the suffix introduced
after the mark, so taking an unrelated main-control savepoint never clones the
retained format vocabulary.

The probabilistic canonical identity used for optional suffix adoption remains
optional for ordinary snapshots. Incremental accepted-history sinks request it
while every retained named boundary's `Universe` is live, and the resulting
identity is stored with that checkpoint. Later convergence compares the two
retained identities directly. It never forks or rolls an accepted generation
back merely to reconstruct an earlier identity.

Environment cells maintain a commutative accumulator of domain-separated
canonical `(semantic cell key, semantic value)` atoms. Each value projection
resolves token, macro, glue, font, and node handles into canonical referenced
content before it reaches the accumulator. The current-cell lookup beside the
accumulator is non-authoritative and proportional only to live non-default Env
cells; it retains neither replaced values nor generations. A typed mutation
subtracts the former atom and adds the new atom, so insertion order and
physical handle choice do not affect identity. The same `CellId`
assignment-scope stripping operation canonicalizes dependency and accumulator
cells.

An environment snapshot retains only the accumulator's fixed-width sum, xor,
cardinality, journal cursor, and delta mark. Ordinary mutation receipts keep
the live lookup current. A rollback replays only the replacement deltas after
its mark and truncates that suffix; consuming the last aggregate rollback root
discards the remaining delta log immediately, including while TeX's own group
save stack remains open. Thus the log is bounded by live rollback authority and
never becomes append history. Group exit applies its typed inverse deltas, and
a generation fork copies the current live lookup once with the rest of the
store substrate. Fresh stores and format images seed it with one live Env walk.
There is no persistent path copy, historical-generation registry, graph
compaction, or compactor. Input, page, PDF, and mode projections likewise
traverse only their referenced content. Append-only store lineages and their
derived membership caches are absent from the composed identity.

The session-local aHash comparison composes that environment root with cached
canonical roots for code tables, hyphenation, magnification/font selection, page-builder
collections and persistent node forests, live input, virtual streams and World
scalars, interaction mode, and the append-only PDF ledger. The fixed-size page,
input, stream, code-table, hyphenation, and font-selection projection cache is
retained in each snapshot and restored with rollback, while journal scratch
remains transient. Root-key comparison is the invalidation barrier: unchanged
roots compose in O(1), and only changed roots rebuild their projection. PDF
state uses rolling semantic
fingerprints and future allocation cursors. One versioned, domain-separated,
fixed-seed 64-bit aHash checkpoint identity is stored on each accepted
incremental boundary and on each new boundary emitted for comparison.
Full mutable-store and page DTO serialization is not part of the session-local
aHash comparison, so unchanged roots are O(1) and work at a compared boundary
is proportional to roots dirtied since
their cached projections. Detached effects and artifacts remain splice-owned
history and are deliberately excluded.

The copy-on-write hyphenation root also retains derived per-language dependency
fingerprints for bounded pure-query cache validation. Pattern, exception, and
saved-code writes invalidate the new root's projection before mutation; forks
of an unchanged root share it. This acceleration is excluded from format
serialization and semantic equality. It is not a retained paragraph record or
a paragraph-specific restart authority.

This identity is session-local acceleration state, not a durable content or
persistence identity. Equality is authoritative for suffix adoption: there is
no SHA-256 or structural fallback on this path. A 64-bit collision can therefore
cause incorrect reuse; that very rare risk is an explicit performance tradeoff.
The fixed seeds make forks and rollback deterministic within one compatible
build/session, while the schema/domain version defines compatibility and must be
bumped when the framing or hash contract changes. Durable `ContentHash` values
for files, fonts, formats, and persisted artifacts retain their cryptographic
identity contracts and are merely framed as inputs where needed.

Snapshots are not public restart points. `tex-exec` alone may publish complete
`EngineCheckpoint`s at `JobStart`, eligible `OuterParagraphEnd`, and outermost
`ShipoutComplete`. A checkpoint structurally owns every root needed for later
validation and restoration. The schedule retains paragraph and shipout
checkpoints for `RootDocument` and `UserDocumentInclude` sources, and filters
`ProjectPackageClass`, `DistributionPackageClass`, `GeneratedInput`, and
`FormatInitialization`. The boundary-forming operation freezes the active role
before queued publication. Mechanical safety remains a separate proof and
does not derive a role from group depth, token provenance, or macro names.
Its command summary directly owns one aggregate command root cloned explicitly
at publication; the command timeline owns only a monotonic identity serial.
The live command root remains exclusively mutable. Retained command roots use
private non-atomic ownership because in-session checkpoints are
thread-confined; generation and timeline capabilities retain their separate
atomic owners. The retained executor store reuses serial-validated physical
slots and tracks their exact live indices, so pruning drops unretained roots in
O(live checkpoints) without scanning full capacity, relocating survivors, or
allowing a stale key to alias a reused slot.

Commit moves structural node/content roots, publishes ordered effects and
artifacts, and releases transaction-local history. Failed validation restores
the prior aggregate state atomically and drops rejected scratch owners.

### 9.3 Published output and accelerator ownership

Published output is a detached ownership class, not an engine snapshot. An
accepted output value owns only its materialized `EffectRecord` values,
`CommittedArtifact` bytes and artifact-local source roots or stable recipes,
detached `DviPagePlan` values, and output telemetry. Restartable
`BoundaryRecord` and `EngineCheckpoint` values remain private session history.
Consequently, keeping an accepted output alive after dropping its session does
not retain a `Universe`, generation substrate, store tuple, or revision map.
Prepared revision output stays inside the rollback-capable transaction until
acceptance; dropping or rejecting that transaction releases its effects,
artifacts, plans, checkpoints, and private generation together.

Every engine/session lookup structure has one of these authorities:

| Structure                                                                                                                               | Authority and lifetime                                                                                                                                                                                                                                                                                                                        |
| --------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Token, macro, glue, and structural-provenance resolution                                                                                | Direct typed indexing in the admitted store slot. It performs no content lookup and owns no candidate, weak entry, registry, or per-value heap authority. The next body migration replaces copy-only durable roots with move-only owners under the same store.                                                                                |
| State-hash projections, page-tree projections, font/hash fragments, hyphenation dependency fingerprints, and editor line/layout indexes | Rebuildable values keyed by exact immutable roots or the current layout generation. They are absent from semantic identity; a miss recomputes the same projection. Fixed-size root projections may travel with a checkpoint, while variable query indexes are charged to their owner.                                                         |
| Pretolerance, page, and shipout pure memos                                                                                              | Detached, handle-free results under explicit entry and retained-byte limits. CLOCK eviction and explicit full eviction change only operational counters and future hit rate. Eviction-key telemetry is bounded by both limits.                                                                                                                |
| Incremental boundary history                                                                                                            | Explicitly charged to the checkpoint/history byte budget. The newest live restart root is protected and reports any unavoidable overage; optional paragraph and shipout boundaries are pruned deterministically. `JobStart` is an independently charged frozen image, not a live history root. History is never copied into published output. |
| Rendered-source page maps                                                                                                               | Rebuildable accepted-output query caches under a dedicated retained-byte limit. Over-budget pages are lowered ephemerally; eviction changes neither source results nor artifact/DVI bytes.                                                                                                                                                    |
| Artifact render provenance                                                                                                              | Detached exact output: artifact-local structural roots or stable editor recipes admitted under the provenance recipe budget. This is not a cache and survives generation release.                                                                                                                                                             |
| VFS input/resource lookups and validated font/PDF resources                                                                             | Authoritative current-generation host bindings, bounded by file/count/byte limits. Superseded private generations disappear on rollback.                                                                                                                                                                                                      |
| Render documents, patch plans, HTML assets, and returned memory output                                                                  | Detached output under render, patch, resource, and aggregate output limits. An acknowledged target replaces its predecessor; these values own no engine state.                                                                                                                                                                                |
| DVI/render/hash builders and temporary lookup maps                                                                                      | Operation-local scratch. They vanish when their builder, candidate, or transaction ends and never cross publication.                                                                                                                                                                                                                          |

Control-sequence interning, current Env/page/PDF maps, loaded fonts, active
resource admission, and current VFS bindings are not accelerators: they are
reachable semantic or host state and follow their owning state budget. No
cache supplies liveness, changes iteration-visible output order, or enters
state/artifact identity. Clearing every rebuildable session cache may increase
later work but cannot change state, diagnostics, resource decisions, source
answers, effects, artifacts, or serialized bytes.

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

One admitted `Universe` is single-owner mutable state. The session store uses
one coarse mutex only to admit a retained slot across host turns; ordinary
value reads borrow the already-admitted universe and take no lock. Parallel
work uses separate session stores or immutable detached artifacts.

### 10.7 Future JIT

A future `tex-jit` may consume a sealed layout surface, but it must use the
same write barriers, generations, effect capabilities, validation, and
deoptimization rules. Until that crate exists, no raw layout API is exposed.

## Verification and performance requirements

- `cargo test --tests` is the hermetic default-native correctness gate.
- `scripts/check.sh` enforces formatting and clippy boundaries.
- Snapshot retention and scaling use `scripts/check-snapshot-budgets.sh`.
- Long-session ownership uses an exact live-owner census at stable equal-work
  milestones; weak metadata and process RSS are bounded diagnostics and never
  semantic or reachability authority.
- Exact fixture and corpus parity defines semantic compatibility.
- Performance changes use the retained state, execution, and whole-engine
  workloads; historical prototype benchmarks are not permanent gates.
