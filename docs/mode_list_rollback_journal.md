# Mode-list rollback journal

Status: production rollback representation

Scope: canonical aggregate operations in `tex-exec`. This document does not
change TeX semantics or the atomic boundary in
[Stepwise Execution](stepwise_execution.md).

## Measured problem

Historically, `StepSnapshot::capture` cloned `ModeNest` before every atomic
operation. Its `Arc` roots made capture cheap, but retaining the old mode-list
root forces the first successful mutation to copy the complete `Vec<Node>`.
The `umber2-johp.298` symbolized profile attributed 5.79% self time to that
copy below `Arc::make_mut`, with 4.24% through ordinary character append and
1.39% through horizontal-run reconstitution.

The historical `mode_list_rollback` benchmark isolated the lifetime with 16,384
pre-existing nodes and 1,024 successful appends. Its length-watermark case is
the lower-level operation an append-aware journal should reach. It deliberately
does not claim that a watermark alone is a complete rollback representation.

Issue `umber2-vgjr.18.4` retired that synthetic comparison after a fresh
caller and authority audit. This document, the production inverse-journal
tests, the mutation-boundary gate, and semantic comparison are the surviving
authorities; the measured source and invocation remain recoverable from Git.

## Required representation

One aggregate mode savepoint is a private, generation-checked cursor into an
undo log owned by `ModeNest`. Each frame records:

- the mode-level depth and identity at entry;
- each existing level's node length and all non-node `ModeList` fields;
- inverse records for removed, replaced, or mutably borrowed nodes;
- inverse records for level push, pop, and replacement; and
- the enclosing frame cursor, so inner commit preserves inverses still needed
  by an outer rollback.

Append and tail extension record only the original length. Scalar setters are
restored from the entry projection. Before an API returns `&mut Node`,
`&mut Vec<Node>`, `&mut AlignState`, or another reference able to change
pre-existing state, its write barrier must preserve the affected old value.
Bulk ownership-taking operations move one coarse source-range carrier through
the aggregate transaction. The journal retains only its owner/generation/range
coordinate and current destination. It never keeps a cloned full-list inverse.
Candidate `PageNodeArena` output is a separate new range produced from an
immutable borrowed source view; reject returns the candidate carrier before
Mode undo, and accepted redo returns the saved source range to its destination.

Commit discards the innermost frame only after merging any inverse needed by
its parent. Rollback validates the cursor before mutation, replays inverses in
reverse order, restores scalar projections and depths, and then truncates the
log. A savepoint is neither cloneable nor externally constructible.

## Atomicity invariants

The existing aggregate ordering remains authoritative:

1. candidate observations, geometry records, boundaries, prepared pages, and
   effects remain buffered;
2. a successful operation commits the mode frame before publishing observers;
3. an ordinary error or typed resource suspension rolls back `Universe` and
   mode state before returning;
4. a fatal TeX82 §81 jump commits partial semantic state exactly as today;
5. nested alignment and box operations use nested frames, never a second
   independent rollback owner; and
6. provenance and effects remain owned by `Universe`; mode inverses store only
   already-owned identifiers and become observable only after aggregate
   rollback has restored the matching `Universe` timeline.

Semantic hashing and named checkpoints read only live mode state. Restart
eligibility proves the sole outer vertical level is empty and quiescent, so a
named checkpoint copies only that fixed scalar level and its optional
demand-maintained identity. It does not retain the operation journal, an
accepted tail, an active builder, or transient mode payload. Journal cursors,
inverse capacity, and spare allocation are operational state and must not enter
semantic equality, traces, formats, or durable summaries.

## Implementation

The typed-boundary phase removed the mutable aggregate escape hatch before any
journal behavior was enabled. Its final production census reduced five
escaping accessor APIs and 189 exact `current_list_mut`/`list_mut`/
`reconstitution_target`/`align_state_mut` uses to zero. Ordinary edits now use
named operations on a private `ModeListMutation` capability. Pre-existing
`Node`, reconstitution `Vec<Node>`, and `AlignState` edits use higher-ranked
closure write barriers that cannot return their mutable borrow. The capability
does not implement `DerefMut`, `AsMut`, or `BorrowMut`, and it has no generic
raw-list mutation closure.

`mode/journal.rs` implements the complete representation. Entry
frames capture stack identities, node-length watermarks, and scalar
projections. Destructive node, reconstitution, alignment, ownership-transfer,
and level operations add generation-checked inverses; append-only operations
add none. Nested commit retains the inverse suffix required by its parent,
while rollback validates the exact innermost frame before replay.

First-write inverse construction is part of each typed journal operation. The
operation checks its projection slot, stores its concrete old value in the
matching typed payload lane only when that slot is unrecorded, appends one
16-byte descriptor to the ordered inverse stream, and then publishes the
stream position. Four-byte scalar payloads stay in the descriptor. Reverse
replay pops the descriptor and its named lane together, so the descriptor
stream remains the sole ordering authority and field marks and frame cursors
remain O(1). The lanes are inline journal owners with warmed capacity, not
per-entry boxes or a second list representation; they are cleared immediately
after the outermost commit and consumed in stack order by rollback. This keeps
the 688-byte popped-level value from inflating every unrelated scalar or list
inverse without adding compaction, a whole-list snapshot, or a retained cache.

On the 64-bit native test target the descriptor is 16 bytes. The payload lanes
are 40 bytes for a page-list root, 128 for test-only alignment state, 64 for an
incomplete fraction, 144 for a display interrupt, 48 for an equation number,
8 for previous depth, 40 for a pending-run projection, 56 for an owned
pending-run value, and 688 for a popped level. Boolean, integer, hyphen-context,
push, and absent-pending inverses add no lane payload. The exact-layout test
pins these values and charges representative records as descriptor plus only
their named payload.

The audited mutation boundary is exhaustive. `push`, `append`, `take_nodes`,
`append_unique_list`, `take_span`, `pop_last_node`, `remove_node_range`,
`with_node_mut`, `with_last_node_mut`, the test-only reconstitution operations,
and display-alignment root transfers use the page-list-root lane. Pending-run
begin, append, and test mutation use absent or scalar projection descriptors.
TFM and OpenType word building borrow the live source run while failure remains
possible; successful retirement then moves its sole owner into the owned-value
lane. A separate destructive descriptor lets an earlier projection replay
after that move-only receipt reinstates the run. Space factor, no-boundary,
hyphen context, previous
depth, previous paragraph lines, incomplete fraction, display interrupt,
equation number, display alignment, test-only alignment state, and mode-level
push/pop each map to their correspondingly named descriptor or lane. There is
no remaining generic inverse-record mutation call site.

The authenticated 20-million-action census recorded this exact first-write
mix: 123,024 no-boundary, 106,816 space-factor, 1,334 previous-depth, 1,142
previous-paragraph-line, 908 incomplete-fraction, eight hyphen-context, and
four display-alignment inverses. At the measured 624-byte historical entry
that was 145,539,264 transferred bytes. The compact structural charge is
3,807,824 bytes: one 16-byte descriptor per call, plus 8-byte previous-depth
and 72-byte incomplete-fraction payloads. The expected reduction is
141,731,440 bytes, or 97.38%, before a new whole-program copy census. At the
direct-chunk/effective-tail integration base, reconstructing the superseded
enum measured 848 bytes because its maximum 840-byte level payload had grown;
the same mix would therefore reduce by 193,976,304 of 197,784,128 bytes, or
98.07%.

Every live `ModeNest` directly owns an enabled journal. Cloning or rehydrating
a `ModeNest` creates a fresh operational journal over the cloned live levels;
journal generation, cursors, log length, and capacity remain excluded from
`Debug`, equality, summaries, semantic hashes, formats, and durable
checkpoints. Aggregate fork and restore are narrower: they construct the sole
rootless outer level from the eligible checkpoint scalar and therefore require
no `Rc<RefCell<_>>`, journal rewind, `split_off`, or forward-tail replay.

Successful ordinary, resource, PDF/effect/output, ErrorStop, observed,
tracked, private-revision, and output-capable box-closing commands create no
aggregate mode savepoint. Preflight settles delivery and scanning before
semantic apply; the authoritative `ModeNest` then mutates directly after
state-layer admission advances the write epoch. Commit advances the
node-operation watermark, while a private revision additionally commits its
fixed-size allocation-suffix mark. Append-only builders retain neither a
cloned prefix nor an aggregate inverse frame. Active alignment and diagnostic
expansion use this same direct path; no production `StepSnapshot` or aggregate
mode retry owner remains. TeX82 §81 fatal propagation commits partial
canonical state before publishing buffered observations and the diagnostic.

## Implementation sequence

The historical 233-seam census included both syntactic mutable accessors and
capabilities forwarded through helper functions, so it was not directly
comparable to the final exact-use census. The source-boundary gate now rejects
the four retired accessor families, mutable aggregate return signatures,
generic raw-list closures, and standard mutable-dereference escape traits.

The first phase replaced direct `&mut Node` access by index and at
the tail with closure-scoped write barriers, then moved every remaining mode
list mutation behind the typed capability. The second phase implemented and
exhaustively tested the disabled journal. The final promotion removed the
retained `ModeNest` clone and compatibility `StepSnapshot`; production now has
one authoritative mode journal.

The acceptance baseline is the integrated five-fixture command-stream report:
`CLEAN`, zero ordered divergences, and zero root sites. The older
eight-divergence/two-root signature predates later canonical fixes and must not
be used for promotion.

## Promotion gates

- Unit tests cover nested commit/outer rollback, inner rollback/outer commit,
  every destructive mutation family, errors, suspension, and fatal commit.
- Observer, geometry, semantic-trace, page/list, effect, provenance, and named
  checkpoint projections are byte-for-byte unchanged.
- The bounded synthetic benchmark demonstrates that successful append cost is
  independent of retained prefix length.
- A symbolized optimized exhaustive profile removes the
  `ModeList`-clone/`Arc::make_mut` caller while the exact tracer report remains
  unchanged.
- The standing native and format/clippy gates pass.
