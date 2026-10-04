# Expansion memory lifetimes

Status: current source map, 2026-10-02.

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
is the authority for checkpoint/group/candidate and definition ownership.
[Runtime storage lifetimes](runtime_storage_lifetimes.md) supplies the other
storage roles. This map does not revive the retired structural-definition
regions, two mutable eqtb target, input inverse timeline, or per-command
resource retry protocol.

Continuing scalar scanners lend their synchronous callback to one shared
scanner-call boundary. The callback allocates nothing and cannot enter retained
state; the scanner boundary enters the same reader as single-command delivery,
without copying that reader for each scanner closure. The single-command
consumer still returns directly. Observation and protection modes remain
static, and all input ownership and unwind rules below still apply.

## Current source and target differences

| Area                  | Current owner/path                                                                                                                               | Required lifetime and remaining work                                                                                                                                                                                                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meaning delivery      | `tex-state` packed dense row writes final `MeaningWord`; `tex-command` selects expansion/return/primitive.                                       | Token and meaning reads acquire no checkpoint or definition owner. A temporary key is valid only while its semantic owner remains, or after full payload admission.                                                                                                                                       |
| Definitions/matcher   | `definition_arena{,/body,/build_lane}.rs`: pinned format rows and stable counted dynamic slots share one immutable words/matcher/origin payload. | Current meanings, independent histories, primitive registry and actual publication owners count roots. Zero roots tombstone a slot; allocation indices never rewind.                                                                                                                                      |
| Macro frames          | Input rows retain `ResidentMacroBody` with full payload `Rc`; position is in the packed frame.                                                   | One admission retain, direct slices thereafter. Payload survives row retirement; target slot tombstoning does not invalidate it.                                                                                                                                                                          |
| Arguments             | `input/arguments.rs` final pooled regions; region-owned ranges and call-local cursor in `macro_call.rs`.                                         | Matching writes words and at most nine ranges once into invocation-owned pooled `ArgumentArena` regions; body owns the region and parameter frames borrow it. No per-argument allocation/copy/lease; free capacity is an engine allocator cache, not snapshot state. Preserve origins and brace trimming. |
| Stored lists/backup   | Input rows own shared immutable list handles or actual transient words.                                                                          | Activation reads immutable storage directly; retirement drops the actual owner once. Backup is semantic input, distinct from host replay.                                                                                                                                                                 |
| Scanner/build scratch | Call-local scanner episodes and nested marks, reusable unpublished lanes.                                                                        | Failed calls unwind/truncate; no parked scanner or caller crosses a host boundary. Genuine scanner requests transfer their token/definition fields directly into native owners; immediate-consumption operation and phase carriers are deleted.                                                           |
| Groups                | One native `GroupSaveStack` in `tex-state/src/journal/groups.rs`; synchronous context/processor unsave and executor renderer.                    | `.6` removes duplicate topology and restoration/token-result vectors. Scalar/box restores and aftergroup words share chronology; sparse saves restore at a lazy marker. The sole boundary has no definition frontier; popped meaning owners survive synchronous restoration observation.                  |
| Named capture         | `CommandCapture`/`InputCapture` in command snapshot code copy actual cold rows and transient words.                                              | Ordinary delivery produces no input undo entries. Actual vectors are charged; capture is not uniformly O(1). Active macros/groups remain forbidden.                                                                                                                                                       |
| Host need             | Typed resource request plus full checkpoint and matched host transaction prefix.                                                                 | Unwind, restore aggregate, admit verified outcome, parse again; no command/matcher/scanner continuation.                                                                                                                                                                                                  |

The removed `ResidentWord`/`TokenMeaning` chain and duplicated font column must
not return as adapters. One interpreted token resolves once into the final
native meaning. Continuing scalar scanners and expanded replacement collection consume native
spelling and meaning synchronously, without a `CurrentCommand` for accepted
words. Explicit command-returning scanners and main control still use their
existing delivery slot while that execution boundary is being removed. Raw macro matching
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

Keyword, optional-equals and numeric-digit consumers remain in the shared
expanded reader across successive terminal commands. Their actual prefix,
value, overflow, and provenance locals live in the synchronous scanner call;
no result tape or scanner continuation is retained. EOF and stored-replay
completion use the reader's existing cold boundary, and every terminal resets
the expanded-delivery fact before the next read. Fuel failures restore both
reader and primitive expansion depths before those locals unwind.

Numeric leading signs, coefficients and internal quantities also enter the
actual scalar consumer directly. Character constants use the raw-spelling
reader, including brace-accounting cancellation. Unit probes and optional space
keep their canonical backups without constructing commands. Number conversion
shares the integer scanner; there is no retained leading command, separate
conversion digit loop or unused radix result tuple. These locals unwind with
the ordinary scanner call and never enter a checkpoint.

Plain resident collection lends the final sink once and admits and writes each
word in one traversal. Argument forwarding splits the source and destination
invocation regions; neither an accepted-prefix pass nor a temporary token
classification precedes the copy. Collectors consume traced spelling directly.
The local scanner cursor owns only brace depth and the outer-group candidate
state used by §394. Paragraph rejection runs at its actual matcher boundary;
there is no cached paragraph flag or counter maintained for every accepted word.

The macro parameter loop owns its one local argument cursor. Both argument
grammars borrow it and mutate the final range in place, returning only success
or error. There is no completed-cursor product or intermediate outer-group
projection. Admission counts the accepted run once; its consuming write updates
the actual final range and virtual raw-word accounting. No accepted-prefix grammar
rescan or invocation-wide parameter-copy stage exists.

Expanded replacement collection lends its actual destination, parameter grammar
and scanner episode to that same reader. Ordinary unobserved macro activation
and ordinary replacement consumption construct no command; only observation
and exceptional expansion actions demand a record. Direct token-list splices bypass replacement brace/parameter grammar,
protected retention and outer recovery run synchronously, and alignment-template
entry stays at its raw semantic boundary. No collection routing/result adapter
or second expansion loop exists.

`expandafter` retains its first operand's spelling, native alignment adjustment
and demanded backup-observation token only. Its second operand is a single-action
consumer of the same reader: native macro activation constructs no command,
failed matching propagates before first-token replay, and no replacement terminal
is fetched merely to complete this operation. All these operands are call-local.

Ordinary unobserved primitives likewise expand from their opcode, traced spelling
and resolved control sequence, without a command record. Their operand recursion
retains only those actual invocation facts. Print values and conditional diagnostic
sites are built on demand; filename and conditional recovery consume native
backup tokens. Observed and exceptional settlement still borrows its genuine
command, then enters the same primitive engine. There is no second primitive
switch, saved command-shaped opener or successful-command cleanup.

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
deferred payload materializer.

Raw `get_token` never expands another macro. Alignment interception only
pushes a template; outer/EOF recovery only inserts ordinary backup/frozen
input. The matcher remains synchronous even if recovery clears scanner status.
One optional capture region therefore owns cleanup across all Result exits,
including conservation failure after completed matching. The result epilogue
restores the actual prior scanner status and releases every untransferred capture.

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

Internal token conversion and expanded replacement splicing consume their
operand in the native expanded reader and keep only the resulting internal
value. Font selection likewise consumes the meaning directly. Their invalid
operand backups belong to the input stack; the scanner callback and resolved
meaning borrow end before replay. Meaning printing takes native resolved facts
only at its demanded cold string consumer.

## Execution provider loan

`CommandProcessor` retains the concrete type of the live `CommandHostFacts`
provider. Initial processors contain only the zero-sized initial facts. The
executor can lend its complete actual execution owner mutably; a native consumer can change its mode, box and driver fields
through `with_execution_owners`, and subsequent conditionals query the same
live stack. Main control owns `CommandState` directly, without an interpreter
wrapper, and both session construction and checkpoint forks initialize their
actual owners exactly once. Cold query helpers may borrow the stack immutably, and genuine
query-only scanner and recursive resource reborrows erase the provider type to
keep their loan shape finite and share the cold reader/scanner specialization. No provider, mode sample, callback or delivery product enters a
checkpoint. `consume_main_control` uses the canonical expanded reader with its
actual main-control role and returns only real stop, replay and EOF boundaries.
The existing execution switch still uses command delivery while its individual
semantic-owner borrows are migrated into native consumption.

The native execution projection also lends the actual capabilities, optional
resource provider and its attempted-resource owner. Source-free semantic helpers
carry an optional observer loan instead of borrowing the concrete operation
buffer. Geometry and nested scanners use that same observer; checkpoints retain
neither loan. Raw aftergroup and afterassignment operands use
`get_token_spelling`, consuming their actual token directly in the canonical
reader. Their save-stack or pending-assignment owner consumes that spelling in
the retained executor loan, without constructing a command or another semantic
machine for the operand.

Macro-argument trace markers and observed token counts belong to their demanded
consumers. Ordinary matching scans and publishes the final cursor without
reading a marker solely for a disabled trace or deriving an unused observation
length. The trace renders the stored definition marker at its actual print
boundary; the observer obtains the final range length before emitting its
ordered delimiter and argument records.

## Retained macro argument spans

The final invocation region owns up to nine compact argument ranges and one
pooled traced-word vector. A sparse retained-source lane contains only actual
immutable definition spans or leases on an original owned argument region.
A source-slot mask identifies those actual backings. An ordinary range names
its pooled word offset; a retained range names its source-lane row. That row
owns the source and its offset, while the range owns the length. Normal short
arguments require no tagged owner or per-argument destructor.

The canonical matcher writes every argument into that final region. Up to four
immutable definition words write directly into the pooled traced spelling, which
costs less than acquiring and dropping a source owner for a tiny span. A single
definition word uses one direct write; only two to four words use bulk extension. A singleton
from another argument also writes directly. A larger contiguous resident span
retains its actual source; contiguous writes extend it. A fragmented write
materializes only its actual argument at the consuming write. Source aliases
flatten once to an immutable definition or original owned word offset. No
second matcher, copied parameter array, deferred-copy stage or successful-command
cleanup stage exists.

Scalar capture queries the live input row only when an actual source lease can
extend. Accepted writes advance the local visible end by their consumed count;
they do not reload the pending range length from the invocation bank.

Parameter input rows name the invocation and argument slot and use local offsets.
Delimiter holdback retains incomplete prefixes; the completing word finishes
the match directly. Brace trimming adjusts the pooled offset or actual source
offset. Tracing, observed values and runaway diagnostics read the same spelling
view, preserving original traced words and unknown origins for definition words.

Input rows can retire before descendants finish with an argument. Retained
sources therefore own their definition body or an indexed lease on the original
owned region. Native release visits only those sparse actual sources, drops
them in place through disjoint bank loans, and iteratively returns zero-root
regions through the already-reserved free list. It neither moves whole argument
values into immediately consumed temporaries nor visits normal ranges to drop
owners they never had. Returned word and source-lane capacity stays with the
engine.

The resident-run reader owns the one transferred stack-local admission cursor. Frame suppression
restricts that actual cursor in place; it does not copy or reconstruct its
configuration. A single match on the actual resident storage lends its definition
body or parameter range directly to the sink. There are no parallel source-kind,
optional-body and optional-region descriptors. Empty-source argument retirement
returns the region directly; only actual retained sources enter the iterative
lease-release loop.

Native parameter and virtual raw-word bookkeeping still counts all logical
arguments and captured words. Full checkpoints and formats exclude active
macro frames and captures. Their cold restore clears the actual transient
owners while preserving reusable pool capacity. Production main control still
returns `CurrentCommand`; removing that handoff is a separate unfinished change.

Macro activation uses the admitted definition directly. No intermediate activation
class is constructed; a contextual check is demanded by parameterless activation
or an empty replacement whose input row might be elided. The matcher saves only
the actual prior scanner status. Its installed matching status is already owned
by live TeX state, and its macro identity remains an invocation fact. Observed
exit transitions derive that status only at their consuming boundary, including
after recovery cleared live status. Other scanner scopes retain the existing episode mechanism, including collectors
that must reassert their installation after recovery. Empty parameter
delimiters have no literal delimiter view or overlap-table projection; the same
canonical argument grammar selects undelimited scanning from absence.

Scanner status is the one actual TeX state, with status-specific diagnostic
owners. Macro matching retains its control-sequence identity; definition and
absorption scans retain their diagnostic target; alignment retains its real
partial builder and owner; skipping retains its starting line and conditional
kind. Synthetic warning serials, a parallel warning column and a duplicated
recovery package serve no consumer and are deleted. Recovery copies the actual
status before clearing it. Scoped episodes retain only that installed status,
the actual prior status and observation demand. Full checkpoint admission
rejects an active status directly, without a redundant warning-state check.

## Continuing conditional skips

TeX82 §494 consumes skipped text inside the canonical raw input reader until
an unmatched conditional delimiter completes the skip. Source words and
resident spans have the same consumer. Literal characters update only actual
brace bookkeeping; control sequences resolve their live meaning once and
update the call-local conditional depth. Ordinary words construct no command,
effect classification, or intermediate skip result. The final delimiter is
the actual semantic result.

Observation, outer recovery, suppression and alignment use a demanded command
inside this consumer. Alignment template insertion continues the same reader;
source retirement, parameter substitution and EOF recovery retain their
existing authoritative input transitions. The consumer preserves get-next
control-sequence creation policy, rather than adopting get-token interning.

The shared reader reserves one fuel charge before entering cold input
transitions. A continuing resident consumer spends that reservation on its
first word and charges before advancing each subsequent word. A call-local
boolean records whether the next word is already charged. Frame exhaustion
and parameter substitution retry under that reservation, so neither transition
adds a charge. Fuel reservation may fail before cursor advancement. Native admission then
consumes the charged word without constructing a per-word error result. Call-local
reservation and nesting facts unwind on failure and never enter checkpoints.
Continuing reservation bookkeeping is selected statically for this consumer.
Ordinary single-word consumers compile the reservation and its callback
wrappers away; both use the same canonical input loop and cold transitions.
No completed-command cleanup or settlement stage is introduced.

## Source spelling ends at lexical consumption

The canonical tokenizer emits ordinary literal characters directly into packed
words and provenance. Its concrete native queries borrow the current state;
no category table is copied or cached. Untransformed control words borrow
validated backing until lookup. Only a transformed name owns its semantic
characters, and the selected name consumer interns it before the scanner call
ends. Native delivery constructs neither a general `SourceToken` nor the
retired `ScannedControlSequence` projection. Lexical inspection still requests
its actual structured source token. Source cursors, normalized lines, origins
and backing retain their existing input/checkpoint lifetime.

## Expansion within character admission

The execution-owned horizontal reader expands ordinary macros and primitives from its already resolved boundary word and continues the same input back edge. Immutable replacement bodies and actual argument regions retain their canonical lifetimes; no delivered command or intermediate replacement carrier owns them. Lexical inspection retains raw boundaries through its actual consumer's static policy. Both expanded readers and execution-owned character admission share one balanced invocation boundary for checkpoint quiescence, including errors. Observed or exceptional deliveries, undefined recovery and end-template handling keep their demanded command records.

Active mathcode expansion uses its already current native spelling, actual macro/primitive operands and the existing expanded-word backup consumer. The synthetic spelling owns no input row and consumes no new raw charge. Ordinary treatment constructs neither a synthetic nor a terminal command; exceptional evidence and alignment keep their genuine delivered facts. Replacement/argument lifetime and canonical backup insertion are unchanged.

Resident rows store the actual packed frame beside their token owner. The one-field TokenRowHeader wrapper is deleted; policy projections borrow the actual row. Cross-crate inlining of the small checked token-frame initializer avoids temporary frame return/copies during macro activation. Row admission, argument publication, retirement and native checkpoint copies retain their existing ownership and accounting.

The authoritative InputLevel contains one packed frame and one InputStorage union, including its sole boxed SourceSlot. SourceLevel, ResidentTokenRow, ResidentTokenStorage and ResidentSourceTop are deleted. MacroBodyCursor retains the actual immutable definition and final argument region; parameter input borrows that region. SourceSlot advances with the real packed frame, and full checkpoint copies retain only their admitted source or durable/traced word owners. No additional row wrapper, source view, compatibility accessor or native lifetime counter is introduced.
