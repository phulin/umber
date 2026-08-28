# Aggregate checkpoint component contract

Status: normative rooted-lifecycle target and measured pre-refactor evidence,
2026-08-27.

## Scope

This document fixes the ownership and measurement boundary for
`EngineCheckpoint`. It does not authorize a cache, fast path, per-value owner,
root registry, compaction, a third generation, per-checkpoint state bank, or
heap indirection. The completed PDF scalar mark is reused unchanged.

One session owns an accepted prior generation and, only while executing an
edit, one candidate current generation. A checkpoint contains bounded scalar
cursors plus one coarse generation owner. Every live runtime value has one
owner named below; a checkpoint is a reachability root, not another mutable
state owner.

## Aggregate transaction

The accepted generation stays live at its physical head. Starting an edit is
one aggregate transaction, never a collection of family-local restores:

1. Validate every selected mark without mutation.
2. In the fixed owner order below, inverse-rewind the accepted head to the
   rooted mark. Retain every old/new journal entry needed to replay the saved
   accepted delta; do not truncate that delta.
3. Open an empty current suffix and enter `CandidateLive`. Execution is
   forbidden before every owner reaches that state.
4. On rejection, undo only the current suffix in reverse owner order, then
   forward-redo the saved accepted delta in owner order. The source becomes
   the exact accepted head again before it is admitted.
5. On acceptance, discard the saved superseded accepted tail, splice or
   promote the live current suffix, and publish only after every owner has
   promoted. Pruning can then release whole obsolete journal/arena chunks.

The protocol states are `AcceptedHead -> AcceptedRewound -> CandidateLive`,
followed by either `CandidateUndo -> RejectionRedo -> AcceptedHead` or
`AcceptedPromoted -> Published`. No component exposes an independently
restored or independently committed state. Unwind cleanup uses the rejection
path; it is not a second acceptance protocol.

The fixed validation/rewind/redo/promotion order is core, command, mode, page,
hyphenation, PDF, World, dependencies, and source/font. Candidate undo and
partial-acquisition rollback use the exact reverse. Execution counters are
copied only after the semantic owners reach `CandidateLive`, and publication
metadata changes only after `AcceptedPromoted`.

### Boundary-record ownership

The executor sidecar stores boundary evidence and its optional move-only
`EngineCheckpoint` together in one typed `BoundaryLane`. The lane uses a
caller-owned `ChunkPool` and one-cell `ForkArena` records. Each stable
owner-relative key is the record's list coordinate plus its sealed
whole-chunk mark; a logical revision number is evidence, never physical owner
identity.

At edit start the selected prefix remains in the same physical arena. The
exact later accepted suffix becomes `detached_prior`, and candidate records
append only to `current`. Rejection drops current records and reattaches the
prior suffix. Acceptance drops the prior suffix and promotes current records.
Neither path clones a prefix, publishes replacement keys, or rebuilds an
index from all boundary rows. A repeated acceptance therefore leaves every
unchanged prefix key and the `EngineCheckpoint` marks stored in its cell
physically valid.

Lookup binary-searches the position-ordered one-cell lane and resolves an
exact boundary/ordinal within that position. Evidence-only records store the
nearest restart record coordinate when appended, so a long run of completion
evidence such as `ShipoutComplete` falls back directly without a backward
walk over the run. Releasing a restart root clears only the option in its
existing cell; detached evidence remains addressable and no checkpoint
payload moves. Fork-arena lifecycle counters require zero source-record
copies through append, accept, reject, and retry.

The page owner follows the same physical-prefix rule. Selecting a retained
`PageMaterial` mark keeps the accepted prefix frames and list coordinates in
the existing timeline, detaches only later frames/inverses, and settles that
suffix on reject or accept. World, source, and font fork setup also restores
their demand-maintained identity scalar to the selected mark before candidate
mutation; replaying the same suffix consequently recreates the same semantic
root instead of hashing it twice from the accepted head.

## Component matrix

| Component                                                                     | Sole live owner now and finally                                                                         | Historical baseline capture                                                                                                                   | Final checkpoint mark or root                                                                                                                   | Restore order after complete validation                                                                                                                                         | Checkpoint retention charge                                                                                                                                            | Complete reachable-state identity                                                                                                                                                           |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Dense command environment, definitions, durable nodes, provenance             | `Universe::core` in the current retained-generation slot                                                | `GenerationOwner` plus `JournalCursor`, durable-node cursor, and conservative page-node cursor; no dense-bank clone                           | The same one coarse generation owner and bounded state/journal/arena cursors                                                                    | Acquire coarse owner; replay dense-state journal; transfer command/mode/page roots before truncating durable and page suffixes                                                  | Charge the generation substrate once across every checkpoint that shares it; charge checkpoint-exclusive cursor bytes per record                                       | Include every reachable live cell after resolving macro, token, glue, font, and node values; exclude dead arena suffixes and physical ids                                                   |
| Command delivery, input, parameters, conditions, groups, replay, diagnostics  | One physical `CommandState` parked in the retained generation or moved into `MainControl`               | `publish_summary` deep-cloned `CommandStateRoots` into one private `Rc`; clone aliased that root; restore cloned it again                     | One sealed typed-arena frame/list mark plus fixed logical-stack, attempt, source-anchor, and profile coordinates; aggregate roots are move-only | Prevalidate profile, generation, frame/list mark, all logical marks, quiescence, and roots; rewind/detach accepted chunks; reject redoes and reattaches prior, accept prunes it | Charge the caller-owned command chunk pool once, then only fixed mark/profile/source-anchor fields per checkpoint; aliases copy no payload                             | Include profile, future input bytes/cursors, parameters, conditions, groups, replay fences, pending semantic diagnostics, and required source/provenance roots                              |
| Mode nest and open mode lists                                                 | The one direct `ModeNestStorage` owned by `MainControl`                                                 | `ModeNest::summary` cloned every level and its list; the intermediate rooted implementation shared the mutable store through `Rc<RefCell<_>>` | One inline sole-empty-outer-level scalar admitted only by a restart-eligibility receipt; no open-list or journal-tail root                      | Install the scalar outer level and a fresh operation journal before page-arena truncation; retain the job-lifetime maximum-depth counter                                        | Charge the fixed rootless outer-level record per checkpoint; no shared mutable mode owner or accepted/candidate tail exists                                            | At legal boundaries include outer-mode scalars and the demand-maintained root; transient list, pending-character, math, alignment, and nested-mode payload is forbidden                     |
| Page builder, lists, insertions, and marks                                    | `Universe::page` plus the generation page-node arena                                                    | Deep-clones `PageBuilderState`; explicit page handles raise the generation's monotonic retained page bound                                    | Fixed page-builder scalar/list cursors plus the existing generation page-arena bound                                                            | Restore page roots after dense state and before page-arena truncation; truncate only after command and mode roots transfer                                                      | Charge page/node segments once per generation and fixed page cursors per checkpoint; never charge shipped detached artifacts here                                      | Include contribution/current/discard lists, page dimensions/integers, insertions, marks, best break, and fire-up state                                                                      |
| Hyphenation                                                                   | `Universe::hyphenation`                                                                                 | Deep-clones language maps, trie vectors, exceptions, hyphen-code maps, and dependency projections                                             | One frozen initialized pattern-trie root shared by the generation, plus journal cursors for mutable exceptions and saved hyphen codes           | Validate frozen-root identity and mutable marks; install the frozen root; reverse mutable journals; truncate exception/code suffixes                                            | Charge the frozen trie once per generation; charge journal blocks once while any checkpoint needs them and fixed marks per checkpoint                                  | Include initialized patterns, exceptions, saved codes, capacities that affect TeX overflow, and `patterns_open`; exclude memoized dependency projections                                    |
| World effects, streams, artifacts, clocks, randomness, and host-visible state | The one `World` inside `Universe`                                                                       | `World::snapshot` clones maps and reconstructs effect-root ancestry while sharing several `Arc` vectors                                       | Fixed effect, stream, artifact, input, publication, clock/random, and file-framing cursors rooted in one coarse World generation                | Validate forkable/retained ancestry; restore effects and stream buffers; restore artifact/input/publication roots; then release abandoned suffix owners                         | Charge retained effect/artifact/input blocks once per generation or output owner and fixed cursor bytes per checkpoint; detached accepted output is charged separately | Include every fact that can change future TeX behavior or emitted effects/artifacts, including stream partial lines, clocks/random state, shell policy, publication order, and file framing |
| PDF                                                                           | The one `PdfStateSlot` inside `Universe`                                                                | Allocation-free fixed `PdfStateSnapshot`; image/form payload is one coarse prefix plus a private candidate delta                              | Fixed scalar cursor plus general/color version-lane roots and coarse payload position                                                           | Select the named general/color roots, reset canonical row selections, drop the private version and row suffixes, then truncate the payload delta last                           | Fixed mark per checkpoint; charge packed version events and image/form payload once to their owning lineage and the candidate-private suffix to current                | Include every canonical PDF scalar, row selection, mutable value, color stack, object/order fingerprint, and payload identity exactly as specified in `pdf_backend.md`                      |
| Dependencies                                                                  | The one `DependencyRuntime` inside `Universe`                                                           | Clones a snapshot that shares the changed-at map through `Arc`                                                                                | Fixed invalidation epoch/root cursor in the generation dependency journal                                                                       | Restore after semantic owners and before publishing any observation; no active recorder may cross a checkpoint                                                                  | Charge the shared changed-at root once and a fixed tracker mark per checkpoint                                                                                         | Include changed-at facts only where they affect future incremental validation; active recorder state and telemetry are excluded                                                             |
| Sources and fonts                                                             | `SourceMap` and `FontStore` inside `Universe`; command input owns its live source frames                | Existing scalar `SourceMapMark` and `FontStoreMark`; the historical command-root clone separately retained live backing                       | Keep the fixed source/font watermarks and one coarse generation owner                                                                           | Validate all source/font coordinates and every command/mode/page/PDF font carrier; transfer roots; truncate font runtime, fonts, then sources before arena suffixes             | Charge immutable source/font payload once per generation or exact external carrier; charge only fixed marks per checkpoint                                             | Include reachable source descriptors/bytes and immutable font recipes plus mutable font-runtime state; exclude unused registered suffixes                                                   |
| Execution and fuel counters                                                   | `MainControl`/session-owned `ExecutionBudgetCounters`, command fuel ledger, and TeX job-lifetime maxima | The two revision budget counters are copied into `EngineCheckpoint`; fuel and stack maxima remain outside semantic roots                      | Keep the fixed revision counters only when restart must continue the same configured budget; no heap owner                                      | Restore counters at fork construction before execution resumes; same-generation semantic restore never refunds fuel or TeX high-water diagnostics                               | Exactly `size_of::<ExecutionBudgetCounters>()` per checkpoint; no payload charge                                                                                       | Excluded. These are monotonic operational evidence, not future TeX semantic state                                                                                                           |

All validation is mutation-free. The normative owner order is the aggregate
transaction order above. The accepted rewind retains its forward values and
does not truncate command, durable-node, page-node, source, font, effect, or
artifact storage. A failed validation leaves source and destination unchanged;
a later failure reverses already-acquired owners before admission can resume.

## Optional identity outcome

`EngineCheckpoint` schema 9 deletes the mode-only `mode_hash` and exposes an
optional `ReachableStateIdentity` under its own schema version. Ordinary
capture never asks for the identity. `CheckpointSink` makes demand explicit,
and the incremental history sink is the sole production requester.

The composer has one owner-bound typed input for every semantic component in
the matrix: command summary, mode checkpoint, and runtime-published page,
World, hyphenation, PDF, dependency, source, font, and core roots. It combines
only journal-maintained scalar roots through a domain-framed fixed-seed stream.
Missing any input returns `None`; no owner id, cursor, partial fingerprint, or
traversal is accepted as a substitute. The aggregate layer contains no free
placeholder functions that could later be filled from runtime coordinates.
Until all owners populate their hooks, incremental convergence reports hash
divergence and retains no suffix. This is the required fail-closed transition
state, not a claim that a partial identity is complete.

The integrated PDF owner now publishes an O(1) component root from its fixed
canonical cursor: mutation-maintained semantic fragments and future-relevant
scalars participate, while general/color version roots and undo positions
remain restore coordinates only. Ordinary capture does not request even this
fixed projection. A focused restore test proves that a semantic PDF mutation
perturbs the root and restoring its canonical cursor restores the root.

Mode and page now publish version-1 domain-separated canonical roots from their
authoritative coarse owners. The incremental history session enables their
inline root lanes before job start; ordinary batch owners keep them disabled
and pay no node, token, alignment, insertion, mark, or arena-payload hashing.
Enabled ordered lanes compose accepted/current and contribution
front/prior/back roots in bounded scalar work. Checkpoint capture copies the
mode root and publishes the page root only under exact-identity demand; neither
path hashes a timeline, frame, cursor, owner, arena row, or payload scan.
Command, World, hyphenation, dependency, source, font, and core now publish
demand-enabled canonical roots beside the existing mode, page, and PDF roots.
Selection occurs before incremental JobStart setup; ordinary batch owners do
no added identity hashing. A late selection after an owner has crossed an
untracked mutation barrier remains explicitly unavailable, so the aggregate
continues to fail closed instead of substituting a cursor or owner id.

The focused perturbation test changes each component root independently and
checks that every resulting complete identity differs. A separate capture
test proves that ordinary capture performs no semantic traversal. The
profiling-only
`checkpoint_identity_gate` compares the two paths after warmup at one and 32
mode levels and requires identical allocation calls and requested bytes. It
also retains an enabled early complete checkpoint across one and 4,096 suffix
mutations, reads the aggregate identity and published mode/page roots 4,096
times, and requires zero allocation calls and zero requested bytes at both
depths.

## Publication-time retention outcome

Every aggregate checkpoint carries an allocation-independent logical charge.
The charge names command, mode, page, World, hyphenation, PDF, dependency,
source/font, and core coarse owners, plus fixed checkpoint metadata containing
the execution counters. Each charge pairs an opaque process-local owner id with
its closed owner-family tag. The incremental publisher keeps the largest
observed byte scalar for each exact `(owner, family)` pair, so a growing
append/journal owner is charged once without merging distinct families or
building a second ownership graph. Command and mode charges come from their
authoritative timeline owners; the aggregate no longer charges only the size
of their checkpoint handles. It charges fixed metadata once for each
restart-capable checkpoint and every detached `BoundaryRecord` separately,
because comparison evidence cannot itself seed restart.

The budget is enforced immediately after each boundary is published. Pruning
first releases restart roots, preferring paragraph roots, and then removes
intermediate detached evidence while preserving JobStart and the newest
observation. Protected JobStart retention and the first/newest evidence pair
may exceed an impossible budget; that overage is reported explicitly. The
accepted-generation transition only validates and prunes the already-bounded
root set, so it no longer hides a larger pre-acceptance peak.

Each observed `(owner, family)` pair also carries a restart-root reference
count. Releasing the last mark into an obsolete lineage chunk releases that
whole chunk, while a genuinely shared append/journal owner remains charged at
its largest observed size until its final restart root is released. This
prevents both multiplying shared storage and retaining charges after pruning.

`RetentionMetrics` reports the shared-owner, per-root metadata, and detached
evidence terms independently, with `checkpoint_root_bytes` equal to their sum.
The retained-owner test checks that multiplying the restart-root count never
multiplies the shared coarse owner.

## Representative baseline

Run:

```bash
cargo run --release --manifest-path benchmarks/tex-exec/Cargo.toml \
  --bin aggregate_checkpoint_baseline
```

The recorded run used commit
`fb81261388a608c16fe26f2cc44084d4574ac2eb`, `rustc 1.93.0
(254b59607 2026-01-19)`, Cargo 1.93.0, release profile, Linux
`x86_64-unknown-linux-gnu`, one process, and the profiling features selected by
the standalone manifest. Fixture construction and capacity growth are outside
the measurement regions. Times are single-run diagnostics; allocations and
requested bytes are deterministic promotion evidence.

| Fixture               | Boundaries | Capture ns / allocations / bytes | Clone ns / allocations / bytes |   Fork ns / allocations / bytes | Restore ns / allocations / bytes |          Semantic checksum |
| --------------------- | ---------: | -------------------------------: | -----------------------------: | ------------------------------: | -------------------------------: | -------------------------: |
| Minimal, 1 unit       |          1 |              26,315 / 29 / 8,967 |             7,720 / 24 / 7,383 |         450,428 / 138 / 351,551 |             27,343 / 29 / 44,887 |     35,406,262,337,344,433 |
| Minimal, 1 unit       |         32 |          224,137 / 897 / 286,944 |        211,523 / 737 / 236,256 |  3,490,032 / 4,416 / 11,249,632 |        167,714 / 928 / 1,436,384 |  6,640,767,404,918,139,627 |
| Accumulated, 64 units |          1 |           62,772 / 469 / 116,544 |          33,925 / 370 / 84,728 |       435,840 / 1,304 / 596,284 |           78,422 / 438 / 151,968 | 12,141,055,338,598,652,146 |
| Accumulated, 64 units |         32 |   3,600,448 / 14,977 / 3,729,408 | 2,532,809 / 11,809 / 2,711,296 | 9,656,388 / 41,728 / 19,081,088 |   2,326,954 / 14,016 / 4,862,976 | 15,155,759,750,561,970,511 |

For the accumulated one-boundary row, the exact fixture payload charges
printed by the binary are: command 133,120 bytes; mode 24,168; page 158,208;
hyphenation 7,168; World 7,168; PDF payload 0; dependencies 1,024;
sources/fonts 2,048; execution counters 16. The PDF mark itself is fixed scalar
metadata; its independent payload-retention and zero-allocation authority is
the completed `pdf_checkpoint_gate` evidence from `umber2-66p0.23.1`.

The 2026-08-27 post-mode/page-identity retention audit reports minimal mode/page
charges of 1,968/528 bytes and accumulated 64-unit charges of 34,704/162,816
bytes. Relative to the recorded pre-identity accumulated row, this is
+10,536 mode bytes and +4,608 page bytes in the existing coarse owners. The
increase is fixed inline booleans/scalar sequence and component roots plus the
bounded mode checkpoint root array; it creates no heap owner, registry, per-node
allocation, or additional generation. Disabled batch owners have the same
layout charge but execute none of the semantic hashing.

At the recorded baseline, capture and restore copied accumulated command, mode, page,
hyphenation, and World state. Checkpoint clone repeats most of those copies.
Fork performs unavoidable construction of one destination generation but also
copies the same accumulated families. The first destination mutation may
currently trigger additional World copy-on-write; this is a deferred
first-mutation copy and is not misreported as capture. PDF payload mutation
already appends to its private delta and copies no prefix.

## World, source, and font ownership outcome

`umber2-pei0.2.4` replaces the World/source/font portion of that historical
baseline with fixed marks and coarse accepted blocks. The concrete ownership
classification is:

| Column or family                                  | Representation at a retained boundary                                                                                                                                | Retained-byte owner and release                                                                                                                       |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Effect records and aligned publication sidecars   | One immutable accepted block per admitted lineage plus an empty destination-private suffix; the checkpoint stores absolute base and aligned scalar lengths           | Each block charges its record payload and six aligned sidecar columns once to the lineage that produced it; a rejected suffix dies with its candidate |
| Publication and semantic counters                 | Current-lineage maps plus one undo journal; the checkpoint stores only the journal length and scalar identity frontiers                                              | Accepted next ordinals are recovered from immutable aligned effect columns, so no counter map crosses or is cloned at fork                            |
| Stream state                                      | Fixed stream-slot state plus effect-position marks; numbered streams retain no unused partial-line mirror because it never changes later TeX behavior                | Terminal/log partial lines and the fixed open/read slots belong to the live World; stream-open context suffixes truncate with their effect positions  |
| Input records and immutable bytes                 | Coarse accepted record/content blocks plus an empty destination-private record/content suffix; the checkpoint stores record and identity cursors                     | Input bytes are charged once to the block containing their record; accepted identities are shared as run blocks and new identities use the candidate  |
| Reduced input dependencies                        | Coarse accepted maps plus a private override map and undo journal; the checkpoint stores the journal length and exact effective count                                | Accepted observations remain in their source lineage block; rollback deletes candidate-only overrides and terminal detachment materializes once       |
| Artifact commits                                  | A retained artifact cursor; a fork starts an empty candidate commit ledger at that cursor                                                                            | Verified/committed bytes cross once into the durable artifact owner; provisional page receipts are quiescent transaction state and cannot checkpoint  |
| Sources                                           | Immutable accepted registration/backing blocks plus private vectors and sparse index for the candidate; `SourceMapMark` remains fixed                                | Descriptor bytes, line indexes, regions, and generated backing are charged once to their accepted block; logical positions and marks are scalar       |
| Fonts                                             | Immutable accepted loaded-font/index/hash blocks plus a private loaded-font suffix; identifier and expansion changes use candidate overrides with bounded undo marks | Loaded metrics and recipes are charged once to their accepted block; mutable identifier/expansion values roll back without copying immutable fonts    |
| Dependency tracker and execution/identity scalars | Revision/invalidation epochs and run-compressed accepted identity metadata                                                                                           | No changed-at map or per-value identity table is cloned; job/revision counters remain fixed scalar state                                              |

Only prior and current lineages are mutable authorities. Accepted blocks are
immutable row storage, not additional generations, and no block registers
roots or compacts/relocates values. Reads select accepted prefix or current
suffix directly; mutation appends to the current suffix or its explicit undo
journal. Restore first validates every mark, reverses mutable journals, resets
scalars, and then truncates suffix columns. A fork shares the exact marked
prefix, excludes source rows after the mark, and allocates only the new
lineage's empty containers.

The enforced `world_checkpoint_gate` compares both 1 versus 64 payload units
and 1 versus 32 retained boundaries. Its recorded optimized result is zero
allocations for capture, checkpoint clone, and no-op restore; World fork is
26 allocations / 1,120 requested bytes at both payload sizes, source/font fork
is 20 / 984, and retained-boundary-only forks are respectively 22 / 888 and
16 / 680 at both boundary counts. First candidate mutation is also flat:
World is 15 / 2,627 and source/font is 26 / 4,706. The fixture reports logical
retained-payload ownership separately (World 848 versus 48,476 bytes and
source/font 2,820 versus 97,068 bytes), demonstrating that real retained data
scales while checkpoint representation and fork setup do not.

## Command ownership outcome

The command family has since passed its standalone promotion gate:

```text
COMMAND_CHECKPOINT_GATE capture=Counts { allocations: 0, bytes: 0 } clone=Counts { allocations: 0, bytes: 0 } restore=Counts { allocations: 0, bytes: 0 } first_mutation=Counts { allocations: 0, bytes: 0 } fork=Counts { allocations: 0, bytes: 0 } fork_first_mutation=Counts { allocations: 0, bytes: 0 }
```

The gate compares one live command unit with 64 accumulated source and
stored-token units, requires identical counts, and verifies rollback plus
repeated-fork isolation. Capture and clone retain fixed marks only. Main-control
drop returns the exclusive command-root loan; fork restores the selected marks
and gives the sole current candidate that root. A rejected candidate returns
the same storage after discarding its current logical suffix.

## Core state, node arena, and primitive ownership target

Dense state and node arenas remain direct mutable owners in one accepted
lineage. A retained checkpoint stores only a journal cursor and reversible
arena coordinates. It never owns a `StateCore`, `DenseState`, or
`PageNodeArena` bank. Multiple sibling marks may name the same lineage without
creating more mutable authorities.

Each dense journal record stores its cell coordinate and one durable alternate
value. Rewinding the accepted suffix swaps old values into the banks in reverse,
leaving the accepted forward values in the same detached records. Rejection
first swaps candidate records backward and then swaps those detached accepted
records forward. Acceptance drops the superseded accepted suffix and promotes
the candidate journal. The checkpoint-delta lane is a typed `ForkArena` over
one caller-owned coarse `ChunkPool`; capture seals a fixed whole-chunk mark and
copies no delta value. Stack and arena journals use the same bidirectional rule
with reversible logical coordinates. Durable values and node payloads are not
copied into checkpoints or reconstructed by replaying page prefixes.

The replacement page-node owner uses one caller-owned `ChunkPool<Node>` plus
typed coordinate-only `ForkArena` lanes. Payload is append-once in fixed-byte
logical chunks stored many per coarse pool page. An immutable pool borrow
returns stable direct node references, while every physical mutation requires
the caller's exclusive mutable pool borrow. The only logical list is normally
one direct `ArenaRange` or,
when composition is necessary, one arena-owned nonrecursive sequence of direct
ranges with cumulative endpoints. Candidate rewind/reject truncates current
payload and descriptor chunks to whole-chunk marks. There is no complete-row
list owner, parallel `NodePiece` stream, linked-node lane, `Vec<Node>` mirror,
recursive rope, overlay, per-node owner, compaction, or per-chunk heap.

An operation mark may include a partial tail and is never retainable. A
checkpoint mark can be created only by consuming live builders and sealing
payload and descriptor tails. Fork ownership is exactly `Accepted` or `Forked
{ prefix, detached_prior, current }`. Reject drops current and reattaches the
saved prior suffix; accept drops the obsolete prior suffix and retains
prefix-plus-current. Pruning releases only explicitly unreferenced whole
chunks; absent a coarse liveness proof, storage remains conservatively retained
until generation retirement.

Prepared output pages follow the same coarse ownership rule in the executor.
One retained-generation sidecar owns a `ChunkPool<PreparedDviPage>` and an
`OutputLane` arena directly. A checkpoint stores only the sealed whole-chunk
mark and page count. Fork moves that sole authority to current after detaching
the selected accepted suffix; rejection returns the settled authority to
prior, while acceptance keeps it in current and drops the superseded suffix.
Candidate drop uses the same rejection settlement. Terminal capture walks the
borrowed canonical rows directly into the final detached page vector, without
an `Rc`/`Arc` owner, mutable accepted tail, parent spine, replay, `split_off`,
or intermediate prepared-page vector.

Long-lived construction uses a move-only `ActiveListBuilder` containing only
the arena owner, its partial operation coordinate, one pending range, and
descriptor-tail scalars. It holds no pool/arena borrow, pointer, or shared
owner. Every push, range append, split, finalize, and rollback presents the
caller-owned pool and typed arena explicitly, validates the builder owner, and
returns before the exclusive borrow ends. An open builder blocks checkpoint
sealing; finalization produces only the canonical range/list coordinate, while
rollback truncates its operation suffix. The builder has distinct vacant,
open, and sealed states and has no conversion to `CheckpointMark`.

The primitive registry remains immutable after initialization. Pruning drops
whole unreachable journal and arena chunks once no sibling mark names them;
it does not scan the engine, register roots, compact coordinates, or perform
per-value ownership accounting. The former bank-loan allocation figures are
historical diagnostic evidence only and are not a promoted representation.

The rooted-lifecycle work in `umber2-pei0.2.13` replaces these independently
materialized checkpoint banks with scalar marks plus one explicit edit-start
rewind/materialization transaction. The identity seam is deliberately narrow:
each owner keeps its demand latch and canonical accumulator, mutation barriers
continue to apply the same old/new keyed contribution or ordered append, and
the future checkpoint mark copies only the accumulator scalar. Rewind and
candidate rejection restore that scalar through the same existing journal or
owner mark. No identity placeholder, checkpoint census, or compensating scan
is needed when `.2.13` removes `StateCore::checkpoint_copy` and the current
World/source/font accepted-block fork path.

## Promotion thresholds

Later ownership-family children must retain this baseline and meet these final
gates without weakening any standing gate:

- warmed capture, checkpoint clone, and same-generation no-mutation restore:
  zero allocation calls and zero requested bytes for minimal and accumulated
  fixtures;
- capture, clone, and restore per-operation allocation counts and requested
  bytes are identical at 1 and 64 accumulated units and at 1 and 32 retained
  boundaries;
- one restart fork performs only one destination-generation construction and
  bounded per-revision setup; its allocation/requested-byte counts are
  independent of accumulated units and retained-boundary count;
- no capture copy or deferred first-mutation copy walks an accumulated prefix;
  mutation opens only the destination's private journal/arena/log suffix;
- median optimized elapsed time per operation across at least 31 samples is at
  most 1.25 times the minimal-fixture median for each accumulated fixture; and
- semantic checksums, cold/incremental parity, PDF scalar-mark gates, routine
  tests, and quality gates remain unchanged.

The time ratio is a promotion diagnostic, not a flaky routine-CI assertion.
The zero-allocation and flat requested-byte assertions are deterministic and
belong in the standalone enforced gate when the final family lands.
