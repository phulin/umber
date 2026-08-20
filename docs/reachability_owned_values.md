# Region-owned runtime values

Status: implemented runtime contract for token lists, macro definitions, glue
specifications, origin lists, and their sparse traced provenance.

This document supersedes the former reachability-owned weak-store design. It
complements [Core Engine State](core_state.md) and
[Private revision allocation domains](patch_allocation_domains.md).

## Invariant

Runtime token, macro, glue, origin-list, and sparse traced-provenance payloads
live in one typed value-region substrate. This region-managed path has no weak
value references, per-value `Arc` marker, refcount garbage collector, liveness
index, graph scan, or parallel allocation authority.

The substrate has two physical parts:

- one bump-allocated mutable candidate containing the current revision's
  append-only suffix;
- sealed accepted regions retained by canonical counted `RegionRootSet`
  values.

An operation mark records the candidate suffix. Rollback restores semantic
destinations and their retained root sets first and then discards the whole
candidate suffix. Acceptance seals selected region storage and publishes a
canonical counted root set; it does not walk individual values to infer
reachability.

## Coordinates and views

`TokenListRef`, `MacroDefinitionRef`, `GlueSpecRef`, and `OriginListRef` are
`Copy` handles. Their generation-tagged ids select compact family coordinates
in the aggregate runtime registry. A successful admission returns a borrowed
view tied to that registry borrow:

- `TokenListView` exposes semantic tokens, cached content identity, and sparse
  traced origins;
- `MacroDefinitionView` exposes flags, the precomputed parameter program,
  parameter and replacement spans, definition origin, sparse traced origins,
  semantic identity, and observation operand;
- glue admission returns the immutable `GlueSpec` row.
- `OriginListView` exposes the exact ordered `OriginId` span and materializes
  structural `OriginRef` values only at a borrowed cold boundary.

Copying a handle never extends storage lifetime. A handle may outlive the
`Universe` that issued it, but it cannot inspect payload after that Universe is
dropped. Admission through the wrong Universe or after rollback rejects the
foreign or stale generation even when the raw slot has been reused.

The live identity of a value is its exact typed generation coordinate. The row
also caches one versioned semantic content identity for format, memo, hashing,
and detached comparison. Physical region, suffix, capacity, and observation
metadata do not create a second semantic identity.

## Aggregate ownership

The aggregate registry is the sole runtime lookup and allocation authority.
Environment cells, journals, command input, macro activations, mode state,
nodes, page state, PDF state, effects, snapshots, and checkpoints store only
copyable family ids or refs. They do not carry a hidden payload owner.

These consumers keep accepted storage live through deduplicated canonical
region-root sets. A durable store checkpoint owns a clone of the sealed root
set admitted at its barrier rather than a scalar publication length. Private
candidate storage is instead bounded
by the active revision and its rollback marks. The important distinction is
between region lifetime and per-value reachability: removing one cell or node
does not perform object collection, while discarding a suffix or a whole
generation reclaims its storage at once.

Command-owned packed token buffers are replay storage, not a value allocator.
When a stored token list crosses into command input, `CommandContext` admits
the source coordinate and copies the traced words into the command buffer.
Active macro records keep only the admitted macro coordinate and command-arena
argument ranges. Replacement and parameter inspection always goes back
through a live `CommandContext` view.

Compact environment and save-stack words are stored keys rather than live
capabilities. A read first rebinds the stored family slot through the current
registry generation and then admits the resulting coordinate. Format glue
maps use the same boundary. Raw reconstruction of a live coordinate from a
stored slot is forbidden because it could bypass stale or foreign rejection.

## Atomic composite publication

A macro row and all data required to interpret it are one region transaction.
Reservation covers the macro record, parameter program, parameter and
replacement token spans, and sparse provenance entries before publication.
Failure exposes none of the composite. Successful publication installs the
macro coordinate and its provenance together, so there is no interval in
which a visible macro has missing traced words.

Token lists follow the same rule for semantic words and sparse origins. Exact
origin lists allocate their identity, location, and provenance-column span as
one registry operation. Glue rows are single-column values in the same
candidate. Empty tokens, empty origin lists, and zero glue are canonical
bootstrap rows, not separately owned sentinels.

## Mutation, snapshot, and rollback

Typed state mutation follows this order:

1. Validate every incoming family coordinate against the aggregate registry.
2. Allocate any new composite entirely in the candidate.
3. Record the ordinary semantic inverse or snapshot mark.
4. Publish copy-only ids into their destinations.

Rollback reverses the order:

1. Validate and construct the command and mode restoration without mutation.
2. Install command, mode, environment, input, page, PDF, and effect
   destinations while the checkpoint still owns its sealed root set.
3. Replace the live canonical root set with retain-before-release ordering.
4. Restore family identity allocators and dense coordinate maps.
5. Truncate the candidate to the saved region mark.

This ordering prevents a restored destination from temporarily pointing into
discarded storage. Raw-slot reuse increments the family generation, so an old
coordinate cannot resolve to the replacement row.

Generation forks built from a durable checkpoint start directly from that
checkpoint's sealed root set. They copy only dense identity/location metadata
and do not first clone the source generation's later mutable suffix. Runtime
handles retain their exact generation when that coordinate is part of the
forked generation; values materialized from detached content receive
destination-local coordinates.

An inherited fork rollback first restores the canonical published root set,
then rebuilds its private candidate from that accepted prefix before applying
the saved family watermarks. The operation mark stores compact coordinate
lengths and the arena generation mark; identity rollback watermarks are
derived only after that generation validates. It does not retain per-family
owner marks or publish roots from private operations.

A checkpoint newer than the selected rollback target may retain a whole
sealed suffix region after its mark has become invalid. Rolling back the live
candidate drops only that candidate owner. Shared sealed storage is neither
unsealed nor recycled; it is released when the newer checkpoint retires. A
recycled slot therefore never aliases a region still owned by an invalidated
checkpoint.

## Formats, memos, and detached continuations

Cold serialization owns bytes and logical content only. It does not own
runtime liveness.

Format encoding writes canonical token, macro, glue, and provenance content
plus collision-safe semantic metadata. Direct lookup tables in the format are
validated serialization structures; decoded tables are not retained as a
runtime value store. Loading validates complete content, reserves region
storage, and publishes destination-local coordinates only after the complete
image is valid.

Detached memo values and `OwnedCommandContinuation` likewise contain
handle-free recipes. Materialization stages names, token rows, macro rows,
provenance, and frames in a destination fork. It commits the fork only after
every reference and bound has validated, preserving atomic failure and one
destination-local identity for each restored value.

## Verification obligations

Tests for this boundary must establish all of the following:

- token, macro, glue, and provenance rows share one candidate lifecycle;
- exact coordinates are `Copy`, and payload reads require a live aggregate
  view;
- stale, foreign, cross-family, and malformed coordinates are rejected;
- equal raw slots from different generations never alias;
- macro words and sparse provenance publish atomically;
- rollback discards a whole suffix and repeated rollback reuses warmed
  capacity;
- accepted regions are retained by canonical counted root sets;
- forks share sealed regions and preserve exact live coordinates;
- format and detached round trips preserve semantic content without carrying
  runtime owners;
- long-lived command, mode, page, PDF, effect, and checkpoint structures keep
  only coordinates and remain safe when the issuing `Universe` is gone.

The source audit rejects reintroduction of `ReachableValueRef`, packed macro
owners, weak token/macro/glue/origin-list stores, or per-value liveness markers
in the region-managed runtime value path.
