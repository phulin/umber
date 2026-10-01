# Runtime storage lifetimes

Status: runtime-wide lifetime policy and current implementation map.

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
is the sole selected authority for generation topology, assignments, groups,
checkpoint intervals, candidates and definition reclamation. Its target is one working mutable core per job/checkpoint family with
accepted/candidate alternates, not one global document engine. Current
checkpoint candidates already move the same dense core between coarse
retained slot wrappers; no second eqtb is created on that path. Necessary
coarse owners, slot admission lifetimes and stable brands remain. The previous
duplicated two-bank, structural-definition-region and command-timeline target
is retired; unrelated session/FFI APIs need no rewrite.

## Semantic ownership

A value's lifetime follows its role. Dense current values, immutable payloads,
execution scratch, node closures, source evidence, output, and host acquisition
must not be forced into a universal lifetime store.

| Storage                                   | Real owner and retirement                                                                                                                                          | Ordinary access                                                      |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| Interned names/spellings                  | Append-only `SessionInternerEpoch`; whole-session retirement.                                                                                                      | Direct stable symbol; never reused within the epoch.                 |
| Dense meanings/parameters/registers/codes | Sole mutable engine; group and checkpoint preimages retain their own semantic values.                                                                              | Direct indexed bank; no overlay/COW/read ownership.                  |
| Macro definitions                         | Target: pinned format rows and counted stable dynamic slots; active frames retain the full immutable payload. Current format/global/local row stores must migrate. | Compact key lookup at admission; frame reads slices directly.        |
| Stored token lists                        | Existing private non-atomic shared immutable payload; last real carrier drops it.                                                                                  | Slice reads; one retain on a real alias/admission, none per word.    |
| Glue and compact provenance               | Generation publisher rows plus exact cold/capture/output owners under specialized policy.                                                                          | Direct typed coordinates; source/provenance is resolved on demand.   |
| Scratch                                   | Reusable unpublished collector/build lanes and ordinary Rust locals.                                                                                               | Direct caller-owned destination; failed calls unwind/truncate.       |
| Nodes/boxes/pages/forms                   | Exclusive page/durable regions, exact history and explicit shared box closures.                                                                                    | Borrowed coordinates under actual region owners.                     |
| Sources/fonts                             | Immutable payloads and direct logical rows/journals; command/input and output retain their actual owners.                                                          | Direct source slot or font row after boundary validation.            |
| World/PDF/hyphenation                     | One mutable authority per component, immutable payloads and specialized undo/version tails.                                                                        | Direct live state; marks belong to the aggregate checkpoint.         |
| Host resource outcomes                    | Verified host cache/VFS readiness outside engine rollback.                                                                                                         | Cold acquisition/admission; ordinary parser replay from a full root. |
| Detached formats/output/memos             | Handle-free bytes or recipes with explicit budgets.                                                                                                                | Cold materialization; no live runtime owner escapes.                 |

Indexed definition counts are explicitly permitted on semantic owner mutation.
They are not a tracing collector, registry, read-time refcount, or atomically
shared eqtb. `DefinitionRef` is a nonowner; a holder that can later dereference
it must have the real root or full immutable payload. No count/log/census is
performed per token or macro replacement word. Global aliases use the same
dynamic slot, so local promotion and group-local storage retirement disappear.
Payload charges follow the final real shared owner, not just arena-row release.

## Interning and immutable lookup

Names survive ordinary groups, replay, and editor revisions in the session's
append-only interning epoch. A symbol always names the same spelling until the
epoch is retired; rejected branches can leave interned names behind. Names are
not definition payload roots. Interning capacity and identity allocators are
host accounting, not rollback-restored TeX values.

Completed primitive registries are immutable session/process lookup metadata.
Typed primitive handles are not mutable meaning cells or definition owners.
Registry completion/format admission validates them once; they never become
format wire identity or checkpoint semantic roots. Fonts likewise distinguish
immutable generated-font identity from mutable TeX runtime parameters; see
[Font identity lookup](font_identity_lookup.md).

### Fresh parameter profiles

Dense state construction is profile-neutral: its fixed parameter banks begin
with zero scalars and empty generation coordinates. Fresh engine construction
then installs exactly one default batch per selected profile layer, without
creating TeX assignment-journal history. The TeX82 layer is the required base;
e-TeX and pdfTeX are optional overlays. Primitive aliases may name the same
physical parameter cell, but the catalogue reduces them to one batch entry so
that every dense cell has one initialization owner.

The TeX82 values follow tex.web §240: all integer parameters start at zero
before `\mag=1000`, `\tolerance=10000`, `\hangafter=1`,
`\maxdeadcycles=25`, `\escapechar=92`, and `\endlinechar=13` are installed.
In particular, fresh `\newlinechar` remains zero. pdfTeX's overlay follows
pdftex.web §§672 and 1064, including its nonzero compression, version,
gamma, origin, pixel-dimension, and ignored-line sentinel values. All other
catalogued scalar cells retain their specified zero value and token/glue
parameters are empty.

A repeated fresh-profile installation is a no-op and cannot overwrite later
Plain or format-source assignments. A restored format never installs a fresh
profile batch: format decoding owns every semantic parameter value and
primitive registration only reconstructs immutable lookup metadata. The sole
post-restore parameter overlay is tex.web §241's job clock. Main control copies
the host-owned `\time`, `\day`, `\month`, and `\year` into the dense bank once
before the first input line and startup banner. Process-selected diagnostic
widths remain operational state outside the format and dense banks.

## Construction and scratch

A scanner writes into the eventual owner where possible. Definitions append
once into the live build lane and seal a single immutable payload containing
words, matcher and origin. Format/import staging is cold. Macro arguments are
constructed once in invocation-owned pooled `ArgumentArena` vectors
(`input/arguments.rs`), owned by the body and borrowed by parameter frames.
Matching writes its final payload directly and publishes only range metadata. Empty arguments are
proved at named capture; returned capacities are an allocator cache charged
once and excluded from semantic snapshots.

Ordinary scanner, expression, alignment-preamble and multi-child primitive
phases live on the Rust call stack. The caller owns already-scanned operands;
an unavailable resource unwinds them. Existing synchronous nested scratch
marks can preserve scanner cleanup, but no mailbox, loan registry, retained
scope tree, or suspended parser is part of the target. The removed
`roll_attempt_operation` path must not be recreated.

Admission validates generation/profile and owner coordinates at cold or typed
boundaries. Within an admitted borrow, values resolve through direct typed
indices or immutable slices. Engine/semantic crates forbid unsafe Rust;
`tex-dense-prefix` owns its isolated allocation/prefix operations, and the
profiling allocator owns allocator forwarding. Neither can become semantic
pointer or lifetime authority.

## Node lifetimes remain specialized

[Node-region ownership](node_region_ownership.md) fixes exact paragraph
restart, page/durable region lifetime and TeX move/copy semantics.
[Node word arena](node_word_arena.md) owns physical words/annexes and
[Dense fork-arena superblocks](fork_arena_dense_prefix_emplacement.md) owns
physical allocation; physical chunks do not define semantic scope.

A unique destructive box/unbox operation transfers an exclusive owner.
Explicit copy and a consuming move whose source is retained by history copy
only the required closure. Actual frozen shared boxes use
[Shared box closures](shared_box_closures.md), not a new node refcount scheme.
Generated paragraph/alignment ranges follow
[Generated box ownership](generated_box_ownership.md).

Box dimensions mutate the existing root scalar without changing binding scope;
checkpoint binding inverses precede lineage-specific scalar inverses.
[Durable box root mutation](durable_box_root_mutation.md) retains that rule.
Per-command transaction frames in current mode/box code are migration storage;
full-checkpoint resource replay does not require them around every dispatch.
The redesign removes redundant operation histories while preserving real
construction/transfer atomicity and root-before-suffix ordering.

Shipout constructs genuinely derived nodes in separate reusable scratch,
borrows source nodes, and detaches handle-free output once. History-owned page
regions release when their last checkpoint interval retires; held-over nodes
must evacuate to their next actual owner first. Never infer region liveness
from an unowned list coordinate.

## Provenance and detached boundaries

[Source spans and provenance](source_spans_and_provenance.md) specifies compact
encodings and [Structural provenance](structural_provenance.md) specifies
semantic ownership. Forwarded argument tokens retain their original source
locations through nested macros. Generated tokens retain their derivation;
diagnostic and output evidence remains resolvable after frame retirement and
restore. Provenance must not accidentally retain otherwise-dead definitions.

Format capture is an explicit cold semantic traversal. It serializes reachable
values as handle-free rows/recipes, including token/node/PDF roots; it is not a
runtime liveness census. Atomic decoding validates schema/profile/fingerprint
before publication. See [Frozen format](frozen_format.md). The immutable
JobStart image owns its bytes and metadata; it is not another live engine.

Outputs, memo records, portable continuations, messages, and host resource
requests contain owned bytes/recipes/locations, never an engine reference,
generation lease, live arena offset, `Rc` payload, scanner cursor, or journal
mark. A diagnostic location that must cross generation retirement detaches
before its source owner disappears. Budgeted operational caches validate
semantic stamps and evict by byte/entry limits; they cannot define semantic
value liveness.

## Checkpoint, candidate and accounting boundaries

All aggregate inventories, costs, named-boundary eligibility, epochs, owner
transfers and repeated restore rules live in
[the checkpoint contract](aggregate_checkpoint_contract.md). Capture may copy
actual cold input/conditional rows and transient words. Dense TeX state is
never deep-copied per named boundary. Group saves and checkpoint first-touch
undo have different preimages, share one assignment policy and retain only
actual owners; empty groups push/pop scalar metadata.

Fuel, cancellation state and diagnostic high-water counters remain monotone
through semantic restore. TeX memory usage is maintained by direct scalar
accounting on actual payload publication/final release and node region/row
publication/retirement. Accounting resolves no values and discovers no roots.
Physical capacity is charged separately from semantic live payload and may
remain for warmed reuse. A zero-root dynamic definition tombstones its stable
slot promptly; any active payload frame continues to own and charge its bytes.

Candidates retain accepted alternates in cold owned tails while one mutable
engine executes. Reject/accept/prune validate before mutation, transfer all
state roots before reclaiming backing suffixes, and publish only after every
owner settles. Specialized PDF root selection and region settlement remain;
blanket replacement with token-heap quarantine is retired. Resource replay
follows [Resource-miss checkpoint replay](checkpoint_resource_replay.md) and
retains no partial parser continuation.

The implementation map and deletion sequence are in the checkpoint contract
and [Minimal hot loop](minimal_hot_loop.md). Architectural speed remains the
goal; performance receipts do not substitute for semantic owner proofs or
impose additional arbitrary closure blockers.
