# Expansion memory lifetimes

Status: current source map and required migration, 2026-10-01.

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
is the authority for checkpoint/group/candidate and definition ownership.
[Runtime storage lifetimes](runtime_storage_lifetimes.md) supplies the other
storage roles. This map does not revive the retired structural-definition
regions, two mutable eqtb target, input inverse timeline, or per-command
resource retry protocol.

## Current source and target differences

| Area                  | Current owner/path                                                                                                                                     | Required lifetime and remaining work                                                                                                                                                                                                                                   |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meaning delivery      | `tex-state` packed dense row writes final `CommandWord`; `tex-command` selects expansion/return/primitive.                                             | Token and meaning reads acquire no checkpoint or definition owner. A temporary key is valid only while its semantic owner remains, or after full payload admission.                                                                                                    |
| Definitions/matcher   | `definition_arena{,/body,/build_lane}.rs`: format/global/local rows share one immutable payload with words, sealed `MacroParameterPattern` and origin. | `.5` removes invocation matcher-plan/failure allocation. `.13` still must replace append-only global and LIFO local payload retention with counted stable dynamic slots.                                                                                               |
| Macro frames          | Input rows retain `ResidentMacroBody` with full payload `Rc`; position is in the packed frame.                                                         | One admission retain, direct slices thereafter. Payload survives row retirement; target slot tombstoning does not invalidate it.                                                                                                                                       |
| Arguments             | `macro_call.rs` accepted-token writer and delimiter holdback; input argument activation.                                                               | `.10` writes once into invocation-owned pooled ArgumentArena vectors; body owns the region and parameter frames borrow it. No per-argument allocation/copy/lease; free capacity is an engine allocator cache, not snapshot state. Preserve origins and brace trimming. |
| Stored lists/backup   | Input rows own shared immutable list handles or actual transient words.                                                                                | Activation reads immutable storage directly; retirement drops the actual owner once. Backup is semantic input, distinct from host replay.                                                                                                                              |
| Scanner/build scratch | Call-local scanner episodes and nested marks, reusable unpublished lanes.                                                                              | Failed calls unwind/truncate; no parked scanner or caller crosses a host boundary. Delete immediate-consumption carriers as their real consumers migrate.                                                                                                              |
| Groups                | State `SaveJournal`, durable-box groups, command group/aftergroup payloads; executor restoration rendering.                                            | `.6` replaces duplicate topology with the direct ordered save stack. Render while real owners live; delete `GroupRestorations` and post-unsave borrowed definition hazards.                                                                                            |
| Named capture         | `CommandCapture`/`InputCapture` in command snapshot code copy actual cold rows and transient words.                                                    | Ordinary delivery produces no input undo entries. Actual vectors are charged; capture is not uniformly O(1). Active macros/groups remain forbidden.                                                                                                                    |
| Host need             | Typed resource request plus full checkpoint and matched host transaction prefix.                                                                       | Unwind, restore aggregate, admit verified outcome, parse again; no command/matcher/scanner continuation.                                                                                                                                                               |

The removed `ResidentWord`/`TokenMeaning` chain and duplicated font column must
not return as adapters. One interpreted token resolves once into the final
native meaning. Actual recovery, tracing, observation and execution consumers
construct only the rich command they require. `CurrentCommand` as a cold
semantic value is not itself an immutable definition root.

## Definitions, aliases and memory

The current sealed payload owns contiguous parameter/replacement words,
validated marker offsets, compiled delimiter failure links and final origin.
A matching/replacement frame retains that payload; it reconstructs no plan or
failure table. Empty/single-token delimiters need no failure allocation.
Overlap links select the reusable literal prefix while the argument writer
continues to own held origins, brace depth, paragraph/outer checks and recovery
under TeX82 §§392--399. No parallel plan store exists.

The target uses pinned format rows and one monotone dynamic bank. Meaning
cells and independent group/checkpoint/candidate preimages use direct indexed
slot roots. Local/global aliasing uses the same key; no definition promotion
copy, local stack mark, promotion memo or per-token counter is needed.
Zero-root slots tombstone without reusing coordinates. Active frames own the
full payload independently, so release follows its final actual owner.
Current row-based memory accounting must migrate to final-payload release;
counting an arena row alone can undercount a still-active body.

Cold `CurrentCommand`, read placeholders, saved restorations and pending
requests must be audited for arena dereference after an owner mutation.
Immediate-consumption restoration/dispatch carriers should disappear. A real
retained consumer instead owns its payload or semantic root explicitly;
copying a `DefinitionRef` is never enough. The authoritative contract names
every owner insertion/drop/swap and the candidate tail migration.

## Input and source retirement

The input frame owns its mutable reader position. Delivery borrows one exposed
frame at a time; the borrow ends before nested activation, replacement or
retirement. No mirrored reader owner/cache or input inverse journal tracks
that position. Immutable source/replay payloads share their actual owners.

Resident exhaustion truncates the logical top once and releases its argument,
macro or list owner once. Source EOF is cold: borrow copy-small finalization
facts, then drop the source slot. Opening a registered future source removes
its backing from the pending map; never-opened registrations have explicit
session lifetime. `force_eof`, retained line number, terminal context and
`every_eof`/template retirement remain genuine TeX facts.

A named checkpoint can own source/stored/backup rows and open conditionals,
with the actual transient words those rows need. Generic clone support for a
macro row never authorizes a checkpoint with a live macro. Command install
clears abandoned scanner/build/replay/group/diagnostic state and retains
session configuration, registered future sources and identity high water.
Candidate command setup/restoration uses real cold captures; its cost is
proportional to their contents rather than per-token work.

## Scanner cleanup and direct construction

Ordinary `def`/`gdef`/`edef`/`xdef` know the assignment destination before
publication. The build lane appends once, validates parameter structure, and
seals one immutable payload. Failure clears unpublished words. Already
detached format/import data may use a cold builder. Dynamic definition scope
will move entirely to the assignment level when `.13` removes namespaces.

Numeric/expression scanners, `read_toks`, alignment-preamble collection,
expanded-definition recursion and multi-child primitives use ordinary
synchronous calls. Saved scanner status/alignment fields restore on every
error before scratch truncates. `read_toks` must not publish a partial target
meaning; multi-child primitives keep already-scanned operands in locals.
Balanced-text destinations preserve each exact optional/repeated phase.
[Structured scanner ownership](structured_scanner_ownership.md) names the
families and their processor authority.

A resource miss unwinds all those locals, matching holdback, builders and
borrowed contexts. The host restores the full aggregate, not the immediate
collector mark. Verified readiness belongs to the host/VFS outside rollback.
Synchronous provider fulfillment can complete the cold call; a declined need
uses [Resource-miss checkpoint replay](checkpoint_resource_replay.md).
Language-semantic input/conditional/alignment stacks remain real TeX state;
they are not resource continuations.

## Provenance and diagnostics

Forwarded arguments preserve original source positions through nested macros;
generated tokens preserve derivation. Definitions, frames, cold captures,
nodes and detached output retain only the provenance their actual semantic
roles require. Follow [Structural provenance](structural_provenance.md) and
[Source spans and provenance](source_spans_and_provenance.md). No provenance
owner may keep a superseded definition alive merely by incidental history.

Diagnostic context is demand-driven. Compact publication coordinates own no
input backing and must validate before rendering. Source/macro scanner
recovery renders synchronously while source, parameters and scratch are live;
terminal/fatal reporting renders before rollback retires the trigger. Page,
output, shipout and observer publication materialize final selector-aware
text at their actual output seam. A host-retained diagnostic detaches owned
text/location before the source generation can retire. No success-path
`String`, decoded source projection or context cache is required.

Group restoration tracing is especially order-sensitive: restoring a print or
trace parameter changes how later restoration entries render. Target unsave
observes each decision with its now-current controls while both live/saved
payloads exist, then drops losing roots. The current created-then-consumed
restoration collection is a migration gap, not an owner exemption.

Diagnostics and observations are transactional output. Completed semantic
queues move their existing allocation to the executor/World rather than
drain-and-collect a second vector. Host replay owns none of those live queues.
Halted execution gates effect sinks; fuel and stack high water do not roll back.

## Cold format, output and retention audit

Formats traverse actual semantic values, node recipes and PDF-only token/node
roots into handle-free bytes. JobStart fallback owns the immutable image and
explicit metadata, not a permanent decoded runtime. See
[Frozen format](frozen_format.md). This traversal is not a hot liveness census.

A definition payload, stored list, input backing or diagnostic result ends at
its final real owner. Reusable vector/build/stack capacity can remain at high
water without retaining dead payloads. Dynamic tombstone metadata remains
stable and is charged separately. Current append-only global definition
payloads are the explicit `.13` gap; command snapshot copies are legitimate
cold retention, not per-token activation history.

Page/box/node/PDF lifetimes remain their specialized exclusive region and
version-root contracts. `.6` deletes redundant group/operation protocols while
preserving real transfers; `.12` audits complete checkpoint restore/drop,
repeatable roots, candidates, resources, provenance and bounded halt. The
migration and proof cases live in
[the checkpoint contract](aggregate_checkpoint_contract.md) and
[Minimal hot loop](minimal_hot_loop.md); no second engine, backwards
compatibility surface, read-time census, relocation or tracing collector is
part of the redesign.
