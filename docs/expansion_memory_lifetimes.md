# Expansion memory lifetimes

Status: current source map, 2026-10-01.

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
is the authority for checkpoint/group/candidate and definition ownership.
[Runtime storage lifetimes](runtime_storage_lifetimes.md) supplies the other
storage roles. This map does not revive the retired structural-definition
regions, two mutable eqtb target, input inverse timeline, or per-command
resource retry protocol.

## Current source and target differences

| Area                  | Current owner/path                                                                                                                               | Required lifetime and remaining work                                                                                                                                                                                                                                                     |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meaning delivery      | `tex-state` packed dense row writes final `MeaningWord`; `tex-command` selects expansion/return/primitive.                                       | Token and meaning reads acquire no checkpoint or definition owner. A temporary key is valid only while its semantic owner remains, or after full payload admission.                                                                                                                      |
| Definitions/matcher   | `definition_arena{,/body,/build_lane}.rs`: pinned format rows and stable counted dynamic slots share one immutable words/matcher/origin payload. | Current meanings, independent histories, primitive registry and actual publication owners count roots. Zero roots tombstone a slot; allocation indices never rewind.                                                                                                                     |
| Macro frames          | Input rows retain `ResidentMacroBody` with full payload `Rc`; position is in the packed frame.                                                   | One admission retain, direct slices thereafter. Payload survives row retirement; target slot tombstoning does not invalidate it.                                                                                                                                                         |
| Arguments             | `input/arguments.rs` final pooled regions; call-local ranges/cursor in `macro_call.rs`.                                                          | Matching writes once into invocation-owned pooled `ArgumentArena` vectors; body owns the region and parameter frames borrow it. No per-argument allocation/copy/lease; free capacity is an engine allocator cache, not snapshot state. Preserve origins and brace trimming.              |
| Stored lists/backup   | Input rows own shared immutable list handles or actual transient words.                                                                          | Activation reads immutable storage directly; retirement drops the actual owner once. Backup is semantic input, distinct from host replay.                                                                                                                                                |
| Scanner/build scratch | Call-local scanner episodes and nested marks, reusable unpublished lanes.                                                                        | Failed calls unwind/truncate; no parked scanner or caller crosses a host boundary. Genuine scanner requests transfer their token/definition fields directly into native owners; immediate-consumption operation and phase carriers are deleted.                                          |
| Groups                | One native `GroupSaveStack` in `tex-state/src/journal/groups.rs`; synchronous context/processor unsave and executor renderer.                    | `.6` removes duplicate topology and restoration/token-result vectors. Scalar/box restores and aftergroup words share chronology; sparse saves restore at a lazy marker. The sole boundary has no definition frontier; popped meaning owners survive synchronous restoration observation. |
| Named capture         | `CommandCapture`/`InputCapture` in command snapshot code copy actual cold rows and transient words.                                              | Ordinary delivery produces no input undo entries. Actual vectors are charged; capture is not uniformly O(1). Active macros/groups remain forbidden.                                                                                                                                      |
| Host need             | Typed resource request plus full checkpoint and matched host transaction prefix.                                                                 | Unwind, restore aggregate, admit verified outcome, parse again; no command/matcher/scanner continuation.                                                                                                                                                                                 |

The removed `ResidentWord`/`TokenMeaning` chain and duplicated font column must
not return as adapters. One interpreted token resolves once into the final
native meaning. The same native `CurrentCommand` remains live through interpreted
raw input, expansion, synchronous scanners and main control. Raw macro matching
and unexpanded replacement grammar consume final spelling and origin directly;
their ordinary accepted words construct no command merely to project those fields.
Recovery, tracing and observation
project spelling, decoded meaning and diagnostic geometry at their actual
consuming boundary. This request-local value is not itself an immutable
definition root.

## Definitions, aliases and memory

The current sealed payload owns contiguous parameter/replacement words,
validated marker offsets, compiled delimiter failure links and final origin.
A matching/replacement frame retains that payload; it reconstructs no plan or
failure table. Empty/single-token delimiters need no failure allocation.
Overlap links select the reusable literal prefix while the argument writer
continues to own held origins, brace depth, paragraph/outer checks and recovery
under TeX82 §§392--399. No parallel plan store exists.

Definitions use pinned format rows and one monotone dynamic bank. Meaning
cells, frozen primitive meanings and independent group/checkpoint/candidate
preimages use direct indexed slot roots. Local/global aliasing uses the same key; no definition promotion
copy, local stack mark, promotion memo or per-token counter is needed.
Zero-root slots tombstone without reusing coordinates. Active frames own the
full payload independently, so release follows its final actual owner.
Payload accounting is charged at publication and released on its final actual
shared-owner drop. Slot capacity is reported separately; aliases share one
physical charge and an active frame remains charged after tombstoning.

Macro admission retains the full payload before argument scanning. Delivered
`CurrentCommand` keys remain call-local; backup stores spelling and resolves
meaning again on delivery. Provisional shorthand tracing renders the old value
before assignment. Unsave owns its popped preimage through the synchronous
observer. Sealing and attempt/read publication return `PublishedDefinition`;
scanner results and native read operands move this actual owner through
reporting, flushing and assignment. Failure drops it. No read placeholder,
namespace promotion or separate accepted definition tail remains.

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
with the actual transient words those rows need. Macro rows have no checkpoint-copy path; admission rejects a live body or
unfinished capture. Returned region capacities are excluded from cold captures. Command install
clears abandoned scanner/build/replay/group/diagnostic state and retains
session configuration, registered future sources and identity high water.
Candidate command setup/restoration uses real cold captures; its cost is
proportional to their contents rather than per-token work.

`CurrentCommand` stores the same packed `MeaningWord` produced by dense meaning
delivery. Its control-family projection selects the direct executor arm;
meaning decoding is demand-local. Scanner token publication preflights the
whole real request before allocating any native destination. It consumes that
request into the same typed shape with native owner fields, without a rooted
adapter, promotion receipt, phase placeholder, or universal operation value.
Diagnostic and nested consumers use the same direct engine and scratch
settlement as ordinary execution.

## Scanner cleanup and direct construction

Ordinary `def`/`gdef`/`edef`/`xdef` publish independently of assignment scope. The build lane appends once, validates parameter structure, and
seals one immutable payload. Failure clears unpublished words. Already
detached format/import data may use a cold builder. Dynamic definition scope belongs entirely to the assignment level.

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

Macro matching owns one local set of at most nine ranges and the current
argument's scanner cursor. Its delimiter plan is borrowed from the retained
immutable definition payload. Held delimiter words are already in the final
region: mismatches reveal them, a successful delimiter truncates them, and
outer-brace trimming narrows the span. Publication copies only range metadata.
There is no scratch argument vector, pending serial, transfer API, or argument
payload materializer.

Raw `get_token` never expands another macro. Alignment interception only
pushes a template; outer/EOF recovery only inserts ordinary backup/frozen
input. The matcher remains synchronous even if recovery clears scanner status.
One optional capture region therefore owns cleanup across all Result exits,
including conservation failure after completed matching. The result epilogue
restores the shared scanner episode and releases every untransferred capture.

Macro-body retirement releases its region after all parameter rows above it
retire. A child capture owns a distinct region, so parent retirement cannot
truncate its unfinished words. Forwarded bulk runs borrow distinct vectors
through `split_at_mut`; destination reallocation cannot invalidate the source.
Resident retirement releases the live row's word/argument/parameter suffix
before dropping that row in place. It does not move the input enum or its
resident payload into a temporary owner. Retirement identity and demanded
observer metadata remain captured before that exact drop boundary.
Backups retain the ordinary input-word lifetime. Tail chains reuse two slots,
with no dead-prefix compaction or second payload copy. Aggregate restore clears
all displaced regions while preserving engine pool capacity. Quiescent
semantic equality/hash ignore vacant slots; physical retained-byte accounting
charges all pool/vector capacities once to the live input owner.

The active unit evidence includes argument publication and delimiter recovery,
forwarding under reallocation/retirement/abort, pool-excluding checkpoint reset,
128-call bounded tail reuse, and every small fuel-cut cleanup in
`input/arguments/tests.rs` and `macro_call/tests.rs`. Existing matcher semantic
fixtures continue to cover paragraph/outer/noexpand/hash-brace and truncated
input recovery; the canonical story and full differential tracer remain
independent TeX oracle evidence.

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
payloads exist, then drops losing roots. `CommandContext::end_group_observed` streams each native action; `CommandProcessor::end_group` admits backup input at that action. No restoration collection survives the callback.

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
stable and is charged separately. Counted definitions release dead payloads promptly; command snapshot copies
are legitimate cold retention, not per-token activation history. Aggregate
admission now preflights command/runtime publication, and public full restore
includes output. Interrupted executor/input/mode owners retire before backing
suffixes, after every selected component passes preflight. Production capture
occurs only after the synchronous driver returns and rechecks quiescence.

Page/box/node/PDF lifetimes remain their specialized exclusive region and
version-root contracts. `.6` deletes redundant group protocols and `.15` deletes operation protocols while
preserving real transfers; `.12` audits complete checkpoint restore/drop,
repeatable roots, candidates, resources, provenance and bounded halt. The
migration and proof cases live in
[the checkpoint contract](aggregate_checkpoint_contract.md) and
[Minimal hot loop](minimal_hot_loop.md); no second engine, backwards
compatibility surface, read-time census, relocation or tracing collector is
part of the redesign.
