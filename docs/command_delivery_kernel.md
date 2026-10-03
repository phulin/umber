# Command delivery kernel

This design moves command-delivery work to its semantic frequency while keeping
one canonical input stack and one expansion implementation. Performance evidence
comes from identical workloads on separate baseline and candidate revisions;
profiling counters are structural evidence, not production timing.

## Input ownership

The semantic input stack is the stack of resumable readers. Each source,
replacement, argument, or stored-token frame owns its logical position and the
physical cursor appropriate to its storage lifetime. Parent readers remain in
their frames while child input is active. Checkpoint capture and restoration
continue to use those existing frame coordinates.

Delivery borrows the exposed frame directly. A second cached storage tag and
its invalidation protocol are unnecessary: the frame's storage variant is the
authority. No Rust borrow survives stack mutation, nested scanning, or resource
unwind. No raw pointer or independent cursor is introduced to evade those
boundaries. Ordinary reads retain the physical macro, argument, and replay
cursors already installed in the frame.

## Recoverable command preflight

Some canonical command checks happen before operand scanning and return a
recoverable error when the current engine mode is unsuitable. PDFTeX's
`check_pdfoutput` and the link-mode check are such boundaries (pdftex.web
§§1535, 1542, 1544, 1548--1552, 1561, 1563, 1601--1607). The cold scanner
backs up the delivered command before returning that error. The backed-up command remains on the actual input stack, with its source cursor
still preceding the operands. ErrorStop may change `\pdfoutput` or the mode,
and the next step re-enters the complete command through ordinary delivery.
There is no per-command rollback owner or operation snapshot.

This is command recovery, separate from host-resource replay. It does not
retain a scanner, expansion frame, or direct-operation mark across a resource
boundary. `\immediate` PDF lookahead has two command deliveries: the command
scanner backs up the looked-ahead PDF primitive, then main control backs up
the outer `\immediate` in the same order required by TeX82 §326. The saved
backup path accepts the outer delivery after nested lookahead has invalidated
its freshness stamp; ordinary `back_input` continues to reject arbitrary stale
deliveries.

## Consumer-directed delivery

Reading and semantic consumption share one input kernel. Source, replacement,
argument, and stored input expose a packed word and frame-local delivery facts;
reading does not resolve a meaning or construct a command. Input transitions
retry under the same fuel charge. Source and synthetic words use that same
consumer boundary, while line acquisition, retirement, and recovery remain cold.
The read facts are synchronous scalar arguments, never a token/result carrier
or a second input owner. A statically selected callback consumes them before
any new input is selected. Immutable stored lists and macro bodies are owned
by their row; transient insertions, backups, and matched arguments share the
input stack's traced-word suffix. Retirement drops the immutable owner and
truncates the owned suffix; aggregate checkpoint replacement releases displaced
rows. Named checkpoints reject active macros.

The consumer is selected at entry. Ordinary text is admitted directly by its
character consumer. Unexpanded definition bodies append packed words to the
existing collector while maintaining brace and parameter state. Control
sequences still resolve live meanings for outer validity; literal characters
need no meaning lookup. Expanded delivery resolves each interpreted meaning
once. A macro supplies only its name, origin, flags, and definition to activation
and argument scanning; successful ordinary activation constructs no command.

Plain resident runs admit and write each word in one traversal. Their sink
borrows the final destination once, updates its grammar and appends the accepted
word directly; there is no accepted-prefix rescan or subsequent span-copy pass.
Forwarded arguments borrow the source and destination invocation vectors as
disjoint regions. They preserve traced spelling while immutable replacement
words acquire unknown origin at that same write. A refused word remains unread.
The frame cursor, fuel and net alignment depth advance once for the accepted
prefix. A source, parameter, recovery or fuel boundary keeps its existing
canonical scalar continuation. Definition and csname sinks use this same run
contract, without a second grammar or intermediate token collection.

Main control's expanded fetch enters this same word loop and one expansion
episode before reading its first token. There is no first-read preflight or
separate first-macro activation path. Main control and scanners differ only at
the selected `end_template` terminal: a directly fetched raw main-control
template inserts and rereads frozen `endv`, retaining its extra raw fuel,
spelling, origin and observations. A template reached after macro/primitive
expansion uses the existing inline terminal conversion, as scanner expansion
does. The loop's actual expanded-state fact distinguishes these cases. TeX's
raw main-loop character lookahead remains its genuine separate fetch label;
its non-character enters this same expanded loop without redelivery.

Meaning resolution writes the final native `MeaningWord` enum: validated static
word, macro key/flags, or font capability. Its native tag and operand are
unchanged between the dense row and the delivered command snapshot. Resolution
stores no derived command class; actual consumers project their class, primitive
or scalar value only when needed. There is no operand adapter, temporary
`TokenMeaning`, or parallel font column.

The hot path has no command-product boundary. Its actual consumer receives the
spelling, native meaning and resolved control-sequence identity synchronously.
The expansion decision is made once from that native meaning; a terminal
consumer must not classify the same word again through a constructed command.
Bookkeeping belongs to the actual mutation: literal braces update alignment
state before consumption, and a source word publishes its diagnostic position
before nested scanning or input retirement.

Observation and exceptional TeX recovery can demand a cold delivery record.
That record supplies the exact suppression identity, alignment correction or
outer-recovery spelling to the actual consumer. It is absent for ordinary
accepted scanner words and replacement tokens. A backup retains spelling on
the real input stack; it does not retain an expanded command. Scalar error
sites decode meaning only when an error occurs. The native consumer interface
must not introduce a renamed command bundle, queued operation, shadow current
command, compatibility adapter or separately classified descriptor.

The execution switch must ultimately consume the same native facts under its
existing processor loan. Removing command products does not remove the real
TeX meaning lookup or opcode decision, and does not change execution, scanner,
checkpoint or group-stack ownership. A resource miss unwinds ordinary calls;
full checkpoints retain the semantic input frames and state owners rather than
an ephemeral consumer or delivery product.

Raw and expanded consumers use the same read, fuel, and input-transition
implementation. Expansion chains stay iterative. EOF, replay completion, and
errors clear outward destinations; every recursive expansion depth is restored
on error. No consumer-tag enum, boxed callback, parked continuation,
parallel reader cache, or alternate recovery engine is introduced.

Scalar scanners consume terminal commands synchronously inside this same
expanded reader until their operand completes. Accepted keyword letters and
leading spaces continue its existing back edge instead of returning through
`get_x_token` and entering again. The ordinary single-command consumer stops
at its first terminal command. Each scanner retains only its actual result,
provenance, and any prefix needed for canonical backup.

Continuing scalar scanners lend their actual callback to one shared synchronous
scanner-call boundary. Each scanner keeps its result locals in its calling
frame and consumes native spelling and meaning directly. Accepted characters
construct no command. Exceptional and observed deliveries lend their demanded
cold record during the callback; rejected ordinary words are backed up from
spelling. The scalar callback boundary prevents each keyword or digit closure
from copying the complete reader into the executable. It has no selector enum,
heap callback, parked continuation or additional input state.

Expansion, raw and expanded observation, alignment admission, provenance, and
fuel still belong to the shared reader. The expanded-state fact resets after
each consumed terminal command. Actual EOF and stored-replay completion use the
existing cold input boundary; consumer locals never enter command state or a
checkpoint.

Conditional skipped-text delivery classifies into the final native meaning;
it constructs no command for discarded tokens.
When observation, command tracing, and alignment interception are absent,
`pass_text` can consume a contiguous resident run of literal non-active,
non-brace characters under one borrow of the existing input reader. Each word
still advances its frame and charges fuel in order. The first control sequence,
active character, brace, parameter, or other boundary returns from that same
reader already advanced and passes through ordinary raw settlement exactly once.
An advanced parameter or exhausted frame enters the same charged input
transition loop as scalar raw delivery. In particular, a parameter in a macro
body must open its argument even while `pass_text` is skipping a false branch;
the argument can contain the branch's `\else` or `\fi` delimiter.
The resulting command also enters the single compact raw continuation for
replay completion and alignment v-template admission.
Source tokenization and exceptional deliveries keep the scalar path. A run
stops after the last available fuel charge so the next rejected token remains
unread, as it would under scalar delivery.

## Native numeric scanner entry

Integer, dimension and glue scanners consume their leading spaces, signs and
first substantive operand inside the shared expanded reader. The actual sign,
provenance and final numeric result live in the scanner call. An accepted sign
or numeric introducer never enters a command slot, and the first substantive
meaning enters the scanner’s internal-value or constant branch directly.
Integer signs, radix introducers and all following digits stay in this same
reader call. The scanner’s actual radix, value and overflow flag are local;
there is no command return or nested digit-reader call after the first digit.
Internal-value scanning borrows the native meaning and original spelling;
coercion, negation and its one observation boundary remain TeX82 §413’s.

Canonical backup points remain real input mutations. Dimension and glue probes
still back up their non-internal operand where §§448/461 require it. Optional
space consumes one expanded token and backs up a non-spacer. Numeric error
sites are constructed from the native spelling and meaning only when needed;
observed and exceptional deliveries preserve their actual correction record.
No reference to an input row or dense meaning cell survives nested scanning.

Decimal text conversion uses this same integer scanner. The separate conversion
digit reader, its command-return loop and its leading-command handoff are
deleted. Overflow, signs, octal and hexadecimal introducers, alphabetic constants,
internal values and terminator handling have one implementation. EOF follows that same scanner’s completion; a resource error unwinds all local
scanner results for replay from the full checkpoint.

## Expanded token collection

Expanded definition and general-text collection use the same expansion reader
as main control and scalar scanners. The collector lends its existing output
owner, parameter grammar and scanner episode to that reader. The reader reads,
resolves and activates an ordinary unobserved macro directly from its invocation
facts; collection must not first create a command solely to invoke expansion,
clear that command and restart a parallel get-x loop.

A synchronous consumer supplies its terminal action and the few actual TeX
collection-specific expansion actions. Ordinary single-command and scalar
consumers have no such interception work; their empty policy compiles away.
The collector intercepts `\the`, `\unexpanded` and `\detokenize` at their
existing direct splice boundary. Token-list splices retain their exemption from
replacement brace and parameter interpretation. Other expansions use the
shared primitive and macro implementation.

Collection retains protected macros with the existing suppression flag and
suppression observation before its expanded-delivery observation. Its terminal
action applies the existing parameter conversion, balanced-brace grammar and
consuming backup in their original order. Alignment-template entry and
`end_template` treatment remain the collection-specific TeX entry semantics;
they must not inherit main control’s frozen-endv reread accidentally.

A failed macro match keeps its original paragraph and outer-validity recovery.
The collector resumes its actual scanner episode at the outer-recovery point,
while ordinary expansion keeps its existing recovery treatment. No callback,
collector continuation or input loan survives resource unwind. Fuel, exhausted
frame retirement, provenance, observations and full-checkpoint replay remain
owned by the shared reader and the actual scanner episode. The separate
expanded collection loop and its routing/result adapters are deleted at the
cutover; unexpanded collection keeps its existing native-word grammar.

## Deferred raw operands and single expansion

TeX82 §368’s `\expandafter` keeps its first operand only as a token for a
later backup. It consumes that operand through the same raw-spelling reader as
macro matching, with source control-sequence creation, outer validation, brace
accounting and alignment admission intact. Its synchronous caller retains the
actual spelling and original alignment adjustment. Observation materializes
only the eventual backup record’s token spelling while the observed command is
borrowed; no full command survives the second expansion.

The second operand is a single-expansion consumer of the canonical expansion
reader. An unobserved macro activates from the resolved invocation facts,
under the existing expansion-capacity guard, then completes immediately. The
reader must not fetch a terminal from the replacement or swallow a failed match.
Ordinary expandable primitives keep their native implementation; an unexpandable
operand is backed up once. The saved first token is backed up only after the
second operand completes successfully, including the original conservation,
brace correction and observation order. An expansion failure unwinds these
actual operands for full-checkpoint replay.

The consumer’s expansion completion and recovery actions specialize statically.
Ordinary expansion continues, expanded collection continues under its scanner
episode, and single expansion completes after precisely one action. There is
no second expansion reader, optional command owned solely for macro activation,
expandable-classification adapter, retained expansion phase, or compatibility
entry for the replaced operand path.

## Primitive expansion from native facts

An expandable primitive consumes its resolved opcode and invocation spelling,
not a fabricated terminal command. The unobserved reader invokes the same native
primitive engine from those facts after the existing semantic-boundary decision.
Terminal consumers receive native facts directly. Observation, suppression,
outer validation, alignment interception and template sentinels retain their
actual cold semantic treatment during the transition.
Native primitive activation preserves the source diagnostic position and each
word’s existing fuel charge before entering its operand scanner.

Primitive engines accept the actual opcode, traced spelling and resolved
control-sequence identity. Most consume only the origin or their nested operands.
Tracing constructs its demanded print value from the known primitive and original
control sequence. Conditional-delimiter diagnostics construct their actual site
only on error, and recovery backs up the native spelling. Input filename recovery
uses that same consuming backup. Observed and already-delivered invocations enter
the same engine from their existing command’s facts; no parallel primitive switch
or compatibility entry remains.

The expanded collector’s direct-splice action accepts the primitive opcode at
this boundary. Its output owner handles only `\the`, `\unexpanded` and
`\detokenize`; all other primitives use the shared engine. Macro activation and
primitive activation both return to the reader’s actual consumer completion:
ordinary expansion continues, collection continues, and a single expansion stops.
No command-shaped primitive descriptor, generic publication product or successful
command cleanup stage is introduced.

## Native completion and exceptional ownership

Each scalar consumer returns its own native completion: a raw delivery status,
an expanded candidate, or the collector's brace result. The shared input loop
maps EOF and replay completion into that consuming type only at a true cold
transition; it does not wrap every delivered word in a second completion enum.

Ordinary word settlement updates the actual source diagnostic position and
literal brace state. Observation, suppression, outer validity and alignment
interception demand the exceptional word owner. That owner captures and
restores the prior diagnostic position for an intercepted delimiter and owns
any semantic error or resource return. Successful ordinary settlement neither
captures the rollback snapshot nor creates a cold error payload before deciding
that no exceptional work is needed. The semantic order remains source-position
update, brace accounting, exceptional handling, and delimiter rollback.

## Observation and settlement

The presence of an external command observer is selected at a delivery entry.
The shared kernel specializes observation-only sequencing, raw observation,
and expanded observation for that choice. The choice is valid only for the
synchronous call: attaching an observer between calls selects the observed
instantiation on the next entry.

Token-local suppression, outer validity, and active alignment remain live
semantic checks. Literal brace accounting remains necessary even outside an
alignment. TeX tracing, diagnostics, fuel, provenance required for recovery,
and checkpoint journals are not optional observation.

## Recursive expansion capacity

Web2C's `tex.ch` change to TeX82 §366 and e-TeX's `etex.ch` [53a]
share one `expand_depth_count`: primitive expansion and `scan_expr` each
enter it, and return restores it. `get_x_token`, sequential macro calls,
and expression parentheses do not themselves add depth. Use pdfTeX's
default limit of 10,000, rejecting an entry that would reach that limit.
Keep the counter in the borrowed processor episode, outside snapshots.
All Rust error returns must restore it, including resource replay and fuel
exhaustion. This corrects the previous expression-only counter.

Native stack capacity is a separate platform constraint. As the Web2C manual
documents, a small native stack can exhaust before the semantic limit.
Routine tests exercise finite nesting and exact boundary accounting with an
already-active parent depth; explicit scaling tests need a sufficient native
stack and must expect a capacity error once the default limit is reached.

Run the explicit full-capacity audit with
`cargo test -q -p tex-command full_default_expansion_capacity_on_a_sufficient_native_stack -- --ignored --exact processor::expand::tests::nesting::full_default_expansion_capacity_on_a_sufficient_native_stack`.
It checks 1,024 genuinely nested scans and the default capacity boundary for
all five operand families on a dedicated 256 MiB virtual native stack.
This is separate from the routine tests of exact boundary accounting.

## Native word completion and cold input ownership

Macro matching consumes final packed spelling and origin directly in its existing
argument grammar. Ordinary accepted words do not construct `CurrentCommand`
merely to project those two fields. The canonical reader still charges each
word, creates source names only within the matching call, publishes source
position before brace settlement, and owns parameter, retirement, EOF and replay
transitions. A synchronous grammar callback consumes each admitted word before
another read can overwrite input state.

Actual outer restrictions, alignment interception, suppression, observation and
scanner recovery construct one command at their cold settlement boundary. That
owner remains borrowed through the same grammar callback when its recovery facts
are needed. Delimiters are intercepted before argument grammar, exactly as in
raw delivery. Paragraph and extra-right-brace errors back up the actual cold
command, or construct a command only for that backup when ordinary delivery had
needed none. Literal brace adjustment, origins, partial argument text, and
backup-before-error chronology stay intact. No alternate matcher grammar or
token descriptor is retained.

The unobserved macro activation shortcut tests its actual macro meaning before
checking whether suppression, an outer restriction or a persistent delivery mode
requires settlement. Other meanings reach their one ordinary settlement call
directly; they do not prepare macro eligibility or repeat outer classification.
Observed and protected delivery keep their existing settlement and terminal
boundaries.

Resident delivery returns only its actual consumer completion or no word. It
never constructs an input-transition descriptor containing row coordinates,
argument regions, parameter spans or source facts. When no word is available,
the actual input owner selects the current top row and cursor after the span
borrow ends. It handles source acquisition, exhaustion and parameter
substitution directly, passing only the operands needed by the owning call.

A replacement parameter stops the resident reader before advancing. Its actual
substitution owner advances the cursor exactly once before attempting the
argument push; a failed push keeps that same consumed position. A final
replacement parameter is substituted before parent retirement. Words stored in
a `MacroArgument` remain raw and do not acquire replacement-parameter semantics.
Fuel remains charged at the existing semantic word admission, not at a new
transition or span charge.

A consumed character prefix returns before an empty or exhausted input owner
retires. Its pending characters and observer metadata therefore retain their
existing publication boundary. Source batching reads the actual top source
owner. EOF, replay completion, diagnostics and resource failures leave through
their real input owner, with no cached transition facts or extra dispatcher.

## Resident character runs

Main control borrows the exposed resident reader once and consumes ordinary
characters inside its storage-specific loop. The existing scalar callback
remains the admission boundary: each character advances input, charges fuel,
and calls the consumer in that order. A consumer stop or fuel failure ends the
borrow immediately. A non-character is loaded once and returns to canonical
settlement; exhaustion and parameter substitution return to the semantic stack.
Only that boundary word receives delivery coordinates. Raw consumers use the
same reader with a statically selected single-word admission policy.

Source-character and balanced-argument consumers already borrow spans. Further
batch admission can build on these owners, but is separate from retaining one
reader selection across scalar callbacks. No consumer borrow survives a stack
mutation, and no copied cursor needs reconciliation with its owning frame.

## Batched skip and argument runs

Two consumers extend the resident character run to control sequences, because
their TeX82 loops inspect only a few facts of each token.

- §494's `pass_text` asks whether a skipped command is a conditional, a
  `fi_or_else` delimiter, or §336's outer class. The skip run resolves each
  control sequence or active character with one dense meaning lookup, keeps the
  nesting count of skipped conditionals inside the run, and settles §347's
  `align_state` for each admitted brace. Only the
  delimiter that ends the skip at nesting zero, outer commands, parameters,
  frozen tokens, and words from `\noexpand`-marked frames are delivered.
- §§392--399's macro argument matcher reads spelling, braces, `\par`, and
  outer validity. The argument run admits a control sequence or active
  character after one outer probe, crosses balanced interior groups while
  tracking depth, and stops at the brace that would close an undelimited
  argument, at a stray closing brace of a delimited argument, at the paragraph
  token, or at the delimiter's first token at depth zero. The selected span borrows its caller’s real scanner cursor: admission reads
  that cursor’s depth and settles each accepted word there once. Appending the
  accepted contiguous words copies storage without a second semantic walk. An
  open definition lends its balanced-body cursor in the same way; admission
  owns no duplicate depth or completed scan facts.

Both runs require that no alignment is active, so no delimiter interception can
apply to a consumed word, and both fall back to character-only admission in
observed episodes and after outer recovery. A run consumes words in delivery
order and charges fuel per word, so a later scalar delivery resumes exactly
where the run stopped.

## Admitted-run settlement

Main control applies consecutive ordinary commands inside one admitted command
context. The actual command slot stays resident across native continuation: a
completed command clears its tag before the next fetch rather than moving an
empty caller option through another local destination. A supplied settled
command skips fetching once; a real character-reader boundary still passes
through the original main-loop lookahead and tracing boundary. Observation
receipts are demand-owned at this processor loan: unobserved native commands
reuse the absent owner, while observed commands return at their real evidence
cut. No per-command empty receipt constructor or destructor is needed. Its
fetch/reswitch loop borrows the initialized caller-owned command
destination itself, rather than moving every delivered command into a second
local. Prefix and reswitch replacements overwrite that same value. A scalar
scanner, backup, diagnostic, or publication consumer copies only its selected
operand when it must survive a subsequent fetch; no reference into input
storage survives a stack mutation. Scalar assignment families consume their
typed scanner outcomes directly instead of staging them through a generic
scalar frame and repeated tuple moves. All synchronous executor scalar families
consume their actual typed scan products; the generic `ScalarScanFrame` union,
status, publication adapters and projections are removed. Nested scanner calls
keep only their own typed value and cold error slots where their actual call
boundary avoids moving the error payload on successful nested reads. Those
slots neither classify products nor retain a scanner continuation. A command settles in place, and the run continues, when it cannot
contribute page material or publish a host effect: the definition, `\let`,
and catcode arms; simple and semi-simple group transitions; §1045's `\relax`;
and rootless scalar assignments (count, dimen, integer, dimension, and glue
parameters, code tables, and `\advance`, `\multiply`, `\divide`) when the
operation is unobserved. Any other command, and any settled command that
published an effect or artifact, returns to the enclosing episode driver.

Between admitted commands the executor only releases the finished command's
attempt scratch. The attempt arena is clear at every command boundary, so a
command that allocated nothing pays one test of its root-table lengths.
Scanner outputs and matching/recovery flags initialize at their actual scalar,
macro-match, and deferred-write boundaries. They are not cleared on unrelated
commands. The processor retains a successfully interned paragraph spelling for
its whole native context loan: assignments can change its meaning but not the
symbol it names. Full restore ends that loan. Actual command trace ownership
and the delivery regime still settle before the next command.

Admission is bounded by remaining raw fuel before accepting a span. Rejected
delimiters, paragraph tokens, outer commands, and closing braces do not advance
the scanner cursor. The final append must succeed before the authoritative
input position, alignment delta, and fuel commit. An append failure unwinds
the synchronous scanner and its unpublished argument or definition owner; no
partially accepted cursor is published or retained for replay.

## Validation

Command tests cover source and resident delivery, nested macro/argument return,
live meaning changes, suppression, raw and expanded observation, input rollback,
and resource/fuel failure. Compare observed and unobserved semantic outputs and
fuel without conflating optional record production with TeX work. Run focused
tests, the native routine suite, and the authoritative `scripts/check.sh` gate.
Report missing pinned assets and pre-existing failures explicitly.

## Assignment ownership and demanded observation

A completed assignment scans, publishes any final immutable payload, and commits
through the same admitted command owners. Integer/dimension parameters, glue
registers/parameters and token registers/parameters require no separate scanner
admission or execution machine. Their switch arm consumes its typed result in
place. Only real mode material, diagnostic publication, resource, output,
afterassignment or observation boundaries release that loan. This extends the
existing native group/count/definition boundary rather than adding another lane.

The assignment committer owns state writes, e-TeX redundant-local decisions and
TeX assignment tracing. Its return is only the actual observable-write decision;
it constructs no mutation receipt, deferred indexed-register product, observer
key, or token presentation. Actual observed consumers construct and publish a
mutation record after a successful observable write, in the existing order.
Unobserved assignments never render parameter names, walk token lists for
observation, or construct observer values. TeX tracing remains an independent
semantic demand, including old token/glue ownership through synchronous tracing.

Token publication validates every scanner root and reserves native destinations
before any allocation or assignment. Publication uses the already-admitted
context rather than releasing and reacquiring the generation. A native alias
transfers its existing immutable owner; a scanned list is published once from
its actual attempt storage. Only successful assignment schedules afterassignment.
The existing native group saves and checkpoint first-touch journals remain the
sole histories; reads and observation absence introduce no journal work.

The expected savings come from deleting repeated scanner/context admissions,
execution-machine setup, receipt construction/transport and unused observer
serialization across an entire assignment family. Completed-document counters
and exact output comparisons establish the aggregate effect; isolated helper
size or a new numerical threshold does not establish completion.

## Token scratch and final definition ownership

Command scratch contains one token row table and one fixed-chunk lane. An open collector and its final list use the same row, with a phase change at sealing; there is no separate result table or word copy. Empty scratch settlement reads one row count. Glue and
provenance already have actual native owners, and a reserved name lane has no
consumer. Their attempt coordinates, parallel origin arrays, mixed promotion
trait and batch transport are deleted. A traced token already carries its
actual origin; scratch must not store a second optional origin beside it.

Both ordinary definitions and `\read`/`\readline` build in the same generation
definition writer and seal once into the final immutable definition bank. The
scanner returns the move-only publication owner directly for assignment.
Runaway diagnostics borrow the incomplete writer; error cleanup returns and
aborts that actual build before checkpoint replay. There is no attempt
definition builder, second word copy, or promotion seam.

An arena has one unique identity allocated when the arena is created. Row
serials advance locally and monotonically within that arena, including across
suffix rollback and command settlement. This rejects stale reused row indices
without a global atomic operation for each scratch row. Token splicing streams a borrowed source once or transfers its entire chunk chain into the parent; it never indexes each word by rewalking chunk prefixes. Serial exhaustion is a
checked allocation failure. Full checkpoint replay installs a fresh arena
identity; it cannot revive coordinates from the halted command.

Semantic routines borrow `CommandContext` directly. A state-free dereference
facade adds no authority and is deleted.

## Select exceptional semantics at dispatch

The native mode and packed command switch selects brace ownership, math-family
assignments, and vertical paragraph entry directly. Those semantics do not run
as a sequence of candidate tests before unrelated commands. A right brace alone
consults the live group and box owners; a left brace alone resolves output,
recovery, and math group precedence. Vertical paragraph entry is a switch case
before operand scanning, preserving the original backup and everypar order.
Ordinary retained commands therefore share the same input and settlement loop
without constructing a semantic projection for irrelevant cases. There is no
second dispatcher, eligibility cache, or alternate execution path.

The mode list and outer observer, alignment, and checkpoint demands are fixed
while the actual processor borrows their owners. Admission reads them once for
that loan; a native semantic call cannot mutate those borrowed owners. Settlement
continues to check live reports, effects, input pushes, afterassignment, and the
caller budget after every completed command. No fact survives an owner return.

## Macro activation call frames

A parameterless macro admits its actual replacement owner and activates it
inside the input consumer, without a separate activation return or the parameter
matcher's call frame. Only an actual parameter
pattern or exceptional observed/traced invocation calls the matcher helper.
That helper owns scanner status, bounded argument ranges, mismatch reporting,
and their error cleanup. Definition admission, simple activation, and matching
share the same native input retirement and body publication authorities. An
empty ordinary replacement retains no exhausted input row or argument owner.

## Cold failure owners in native results

A successful reader or scalar scanner carries its native completion or value.
It does not carry inline filename strings, resource requests, diagnostic work
snapshots, or annotated failure bodies. Command failures keep their small
identity and native scalar variants; every larger payload has one cold boxed
owner, allocated only when that failure is raised. Fuel work, resource failure,
and origin annotation consolidate their previously boxed subfields into that
single payload. Formatting, resource replay, and final fatal conversion consume
the same payload through the actual error boundary. There is no second error
protocol or compatibility constructor.

Execution likewise boxes actual missing-font, missing-image, and World failure
payloads only when raising them. Completed font/image operands stay native and
unboxed. This removes cold request data from every main-control result without
adding bookkeeping, allocation, or a result adapter on the successful path.

## Internal operands and font selection

The shared reader supplies `\the`, expanded replacement splicing and font
selection with native spelling, meaning and control-sequence identity. Each
scanner consumes its operand synchronously and retains only its semantic result.
The replacement collector backs up an invalid internal operand on the actual
input stack before ordinary `\the` recovery. Font rejection likewise restores
the spelling before rendering TeX's error context. Observed and exceptional
inputs lend their demanded cold record without creating one for ordinary input.
Cold meaning rendering accepts the already resolved meaning and identity;
printing an invalid operand does not require a command object.

## Native execution owner loan

Main control’s consumer must execute from native spelling and meaning under
its existing processor loan. That consumer needs the actual mutable mode owner
at a mode change, and expansion or operand scanning may consult that same mode
owner before the change. An immutable erased mode-fact loan cannot support
both operations; a command object must not become a workaround for releasing
that loan.

The processor preserves the concrete type of its borrowed execution facts.
Initial processors use the zero-sized initial TeX facts directly. Executor
processors borrow their actual provider, which can own mutable mode access.
Scalar scans and expansion read the provider’s live fact methods. Synchronous
semantic application reborrows that provider alongside the actual state,
command, fuel, diagnostic and observer owners. This straight-line owner
projection always inlines, so the selected arm does not construct a callback
environment solely to pass those owners through another callable thunk. It creates no mode snapshot,
parallel current-command cache or deferred operation. The execution switch
remains the sole authority for opcode, prefix and mode ordering.

Host-resource providers remain cold and erased. Query-only cold scanners and
nested replay share the erased fact-provider specialization, without copying
facts or creating another engine. The native execution consumer preserves its
concrete owner type; cold helpers do not duplicate the reader and scanner code
for an immutable provider they only query. All of these
loans end on result unwinding. Full checkpoints retain the existing semantic
owners, never a fact provider, native consumer or host capability.

Ordinary group entry/unsave, macro and let assignment, and relax character
flushes now reborrow the real mode owner through this loan. They keep the
existing reader alive across the flush and subsequent native mutation. A
preexisting diagnostic demand still releases the loan for ordered reporting.
Group unsave inserts aftergroup words through that same processor; flushing a
word creates neither a replacement processor nor a cached mode sample.
