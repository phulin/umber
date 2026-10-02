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

A full hot command is materialized when a caller actually needs command delivery,
or when observation, suppression, alignment, or outer recovery requires the
existing settlement boundary. Materialization uses the already-resolved meaning;
it must not repeat a dense lookup. Illegal definition parameters materialize
only the rejected token needed by backup. Macro matching retains only spelling
and invocation facts; prefix diagnostics construct their site directly from
those facts, without a command or delivery coordinates. Origin remains valid
when the original input frame retires.

Raw and expanded consumers use the same read, fuel, and input-transition
implementation. Expansion chains stay iterative. EOF, replay completion, and
errors clear outward destinations; every recursive expansion depth is restored
on error. No runtime consumer selector, boxed callback, parked continuation,
parallel reader cache, or alternate recovery engine is introduced.
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
