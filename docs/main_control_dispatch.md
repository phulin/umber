# Main-control dispatch

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
governs this engine. `tex-command::CurrentCommand` contains its native packed
`MeaningWord`; `control_code` projects its dispatch family without storing a
second decoded meaning. Command identity is derived at its actual diagnostic
or observer consumer from that packed word and the existing noexpand flag;
ordinary delivery stores no redundant identity column. `main_control/direct/dispatch.rs` is the one packed
mode/command switch, including TeX82 §1030's `reswitch`, §1034's horizontal
character loop, and §1211's prefix loop. Ordinary, observed, tracked,
diagnostic, nested math/accent/alignment, PDF, and resource consumers enter
these same arms.

The switch key is a demanded projection of the native word. Scalar opcode
tags occupy one byte’s range, followed by the sixteen literal catcodes,
expandable byte selectors and unexpandable sixteen-bit selectors. These
disjoint small ranges permit ordinary indexed branching; retaining the
opcode in bit 56 would create a sparse 64-bit switch for the same semantics.
The key is local to dispatch. Stored meaning words, font/definition
capabilities, formats and checkpoint records retain their native encodings.

Expanded fetching and raw horizontal lookahead retain their distinct TeX
labels. Borrowed source spans enter the actual horizontal material consumer.
Prefixes and replacement commands dispatch in place without redelivery. The reswitch label derives one local packed switch key from its actual current command, refreshing it when a prefix or replacement fetch changes that command. Scalar scan scratch belongs only to the native arms that scan scalars. Each assignment arm resolves its effective global flag once before scanning; unrelated commands read no GLOBAL_DEFS. Brace and math consumers query the live group only when that semantic case needs it. Disabled tracing prepares and reports no command evidence. Each
selected arm scans its concrete operands through the existing processor. A
source-free native arm reborrows that processor's actual context, command state,
fuel, diagnostic and observer owners while the processor remains live. The callback owns
no payload, phase, counter or separate dispatcher. The processor preserves its concrete execution provider, which lends the
actual mode owner mutably. Native application can reborrow that same owner;
subsequent expansion reads live mode facts. Ordinary groups, macro/let assignments and relax flush pending characters
through that loan and retain the existing processor. Genuine outer semantic
calls still end it where their owner requirements demand it. Context-consuming commands retain the processor and context in the same
fetch/reswitch loop. Actual consumers reclaim their scratch at consumption,
and demanded tracing records own their reporting boundary. Genuine diagnostic,
observer, output, alignment or World demands end the appropriate owner loan.
Each command consumes its real fuel and counts toward the caller's remaining
command budget; successful continuation has no settlement or cleanup stage. Native semantic
arms examine the diagnostics, pending output, observation, tracked-region, alignment delivery and synchronous World-effect
demands they actually produced; those demands end the loan before the existing
publication, checkpoint, page-owner rotation or resource boundary runs. An
error drops the real processor and context before aggregate failure settlement.
There is no precomputed eligibility classification or parallel command loop.
The same packed switch selects context-consuming semantics and genuine outer
owner calls. A declined or exceptional delivered word retains its required site facts in
`CurrentCommand` through its actual semantic consumer; ordinary admitted input,
expansion, scanning and text consume native words directly. Spelling and
printed identity are projected only by their actual consumers. There is no
HotCommand-to-CurrentCommand repacking boundary. Native callbacks return only their actual semantic result. The shared native
continuation routine reads the live page, command, diagnostic and World owners
and updates the save-stack high-water mark in place. It copies no page-output
facts for a command that continues. Only an actual outer cut captures those
facts, before diagnostic and named-input publication; errors use that same
ordered cut. No success-fact tuple or stored continuation predicate exists. A diagnostic program cannot cross the next input acquisition:
nonempty diagnostics finish through their ordered publication boundary first.
`scan_*` and `apply_*` files group concrete routines by responsibility;
`semantic/` holds source-free semantics. The large authoritative switch stays
in one file so its mode and prefix ordering is visible together.

Genuine scanner requests parameterize their token or definition fields by
ownership domain. Token publication validates every attempt root, reserves
all native rows/words, then consumes the request while streaming final token
owners. Alignment templates use the same complete preflight discipline. There
is no duplicate rooted request, operation union, phase placeholder, readiness
mailbox, eligibility lane, prepare/apply protocol, or second executor.
Definitions and let assignments use that same retained processor. A macro
publication remains move-only through pending scan reports and any demanded
character flush, then the central assignment acquires its installed root before
releasing that publication. Let scans its complete RHS meaning before applying
any LHS change, and central assignment acquires the alias before dropping the
old cell. Protected-macro evidence uses the processor's actual observer; mutation
receipts remain demanded and publish through the common ordered tail. Pending
scan reports return the actual semantic result to the selected arm; they cannot
skip its successful afterassignment tail. Failed character flushes settle that
error before returning and never install the publication or fire afterassignment. No
complete command machine is constructed for these arms. Definitions have
counted native ownership; provisional shorthand diagnostics
consume the old owner before replacement.

`semantic_owners.rs` lends actual command, mode, box, alignment, and output
owners to a concrete call. Case-specific code publishes alignment and
assignment facts at their semantic positions. The common tail preserves show
completion, deferred print, shipout, page-output, and paragraph ordering. Native context-consuming semantics return directly without constructing
an unused command machine or entering a command settlement stage.
Observed execution emits its ordered mutation/effect/artifact receipts;
unobserved execution constructs no receipt batch. `host_boundaries.rs` admits
demanded receipts and latches failures. Actual consumers reclaim scanner scratch. It creates
no per-command savepoint or lifecycle capability.

Host misses unwind call-local scanner and semantic owners. The host restores a
full aggregate checkpoint before retry; fuel and high-water statistics remain
monotone. Genuine page, output, candidate, and aggregate owners retain their
existing cold lifetimes. Performance measurements distinguish actual hot work
from unnecessary setup and drive regression fixes; historical instruction
budgets impose no arbitrary numeric threshold.

Replay accounting counts completed driver chunks at the host boundary. A chunk
can contain several native commands; removing cold handoffs changes this
partition without changing replayed state or output. The resource-equivalence
test requires real discarded work and exact restart/response counts while
comparing complete preloaded and partitioned output byte for byte.

## Native execution owner

### Name and conditional relation consumption

`\csname` and numeric conditional relation probes consume native words in the
shared expanded reader. The name collector keeps its final string and appends
resident character runs directly; it creates no per-character command. TeX82
§372 stops at a control sequence even when `\let` gave that sequence a character
meaning. `\endcsname` completes the name, and another control sequence is backed
up before the missing-terminator report captures its input context.

The single relation scanner handles `\ifnum` and `\ifdim`, including their
recursive calls. It returns the actual comparison selector and keeps no command
slot or separate compact scanner. A missing relation captures diagnostic facts
at its consuming boundary, backs the offending spelling with its actual
alignment correction, then reports the inserted equals sign. The common report
routine accepts that real diagnostic site after backup, rather than a command
record that it would immediately unpack and discard.

### Expression and math lookahead

Expression factor and operator probes consume native meanings in the shared
expanded reader. Leading meaning-spaces continue inside that reader;
parentheses and operators complete their actual decision. A rejected token is
backed up immediately from its spelling and, only at an exceptional delivery,
its original alignment and observation metadata. The expression stack and
arithmetic state remain the same owners. No operator command slot or decoded
meaning projection is retained. `scanners/expression/arithmetic.rs` owns the
unchanged factor, term and glue-component arithmetic; the parent module owns
the scanner and its actual expression frames.

Math opening lookahead uses the canonical raw reader, while closing display
lookahead uses the expanded reader, preserving TeX82 sections 1138 and 1197.
Each decides from the actual math-shift meaning, or backs up immediately, and
returns its genuine boolean decision. Control-sequence character aliases are
classified by meaning; backup retains their original spelling.

Canonical backup mutation receives only actual storage, treatment and alignment
adjustment and returns the installed input level. Its callers demand spelling
observations after insertion; no observation factory crosses the mutation.
The cold alignment-correction observer reconstructs the previous state from the
just-applied unit brace adjustment. Input insertion does not modify alignment,
and delimiter adjustments never emit this brace-correction record. Ordinary
backups neither copy observed spelling nor load a discarded previous state.

### Filename consumption

Filename scanning uses continuing native-word consumers for its preliminary
probe and character grammar. Web2C tex.ch [29.526] requires a preliminary
fetch and backup before selecting the braced or unbraced path; that actual
handoff stays in place. Both consumers classify native character meanings,
retaining only the final components, quotation/group grammar and the first
origin used by the result. A terminating noncharacter is backed up immediately
with its original spelling and any demanded exceptional delivery metadata.
No per-character command or decoded-meaning projection is constructed.

The filename guard and actual scalar-error unwind surround this one scanner.
String-pool publication remains after character collection and before the
completed filename returns. Source opening and resource replay keep their
existing outer ownership boundary.

### Restricted scalar values

Restricted integer scans return the actual recovered integer and its genuine
primary origin, using the same scalar value as ordinary integer scans. The
separate restricted result and unused unrecovered-value/recovery fields in
character definitions, math requests and input-stream requests are deleted.
TeX82 sections 433 through 437 still diagnose from the original scanned value
before returning the canonical zero. The ifeof observer's extra bad-stream
record is demanded locally from its original operand; unrelated scans carry
no recovery flag for it.

Profile register selection chooses the actual eight-bit or extended bound
once and invokes integer scanning and canonical range recovery directly.
Eight-bit and explicitly extended entry points retain their real callers;
there is no compatibility alias or second range-check implementation.

### Raw binding operands

Binding scans return the actual native `MeaningWord` that the eqtb installs.
`\let` neither decodes its RHS nor reencodes it during assignment. e-TeX's
redundant-local-definition check compares the two native words directly;
observed mutation rendering decodes only at its demanded receipt boundary.
The admitted context's meaning write accepts that same native word, validates
its font capability when present, then enters the existing central state
mutation. All callers use this one API; the resolved-meaning write API is
removed. Definition publications acquire their installed native root before
releasing the publication, preserving group and checkpoint ownership.

The name terminator check compares the packed word to its canonical encoded
`\endcsname` constant. It does not decode unrelated meaning variants.

The canonical raw reader lends spelling and, when its consumer requests it,
the already-resolved native meaning. A spelling-only consumer does not resolve
ordinary literal characters. Continuing consumers keep accepted spaces and
the optional equals sign inside that reader; no delivered command or operand
packet crosses the successful boundary. `get_token_spelling` returns the
actual retained token when its caller needs one.

The futurelet RHS owns a separate outlined grammar: its retained first token,
backup metadata and second-operand reader frame do not enter the normal let
call frame. Both grammars return the actual native meaning and share target
scanning and canonical input mutation. There is no second let API or result
wrapper.

Normal `\let` consumes the RHS meaning directly. Its leading spaces are
meaning-based, while its optional equals sign is spelling-based; after that
sign it skips at most one space. `\futurelet` retains only its first spelling,
actual alignment adjustment and demanded observed spelling, then ends that
reader call before reading the second token. It backs the second and then the
first and returns the second meaning. `\expandafter` retains the same real
backup facts across its second operand. All backups share one input-stack
conservation, alignment correction and final-input insertion implementation;
their demanded observations follow insertion. Exceptional alignment, outer
recovery and observation lend their existing command only at those demanded
boundaries. No successful backup constructs a replacement command.
The three raw operands that must later be backed up (`\futurelet`'s first
token, `\expandafter`'s first token, and `\noexpand`'s target) share one
reader entry returning those actual backup facts. `\noexpand` restores its
temporary normal scanner status before backing the target with suppression.

Main control separates the owned `CommandState`, fuel, resource capabilities,
operation observer and resource-decline slot from one owned `ExecutionState`.
That execution owner contains the existing mode, box, alignment, math,
paragraph, output, job and driver fields. This is a partition of the real
fields; it introduces no sampled facts, copied roots or additional semantic
state. Neither owner dereferences to the old flat control layout.

The reader borrows its own owners and the actual execution owner independently.
A native terminal consumer reborrows the latter with the admitted state and
command owners. The existing mode/prefix/reswitch switch remains the only
execution authority. It must perform operand scans and semantic mutation at
that consumption point, rather than returning a command product, an unbundled
command tuple, a queued operation or an eligibility handoff. Actual output,
resource, checkpoint and diagnostic demands stop the reader and return their
real result after its loans unwind.

The executor fact interface reads the execution owner's actual mode/list state.
It retains no sample across execution. Source-free routines borrow the exact
execution and command fields they need. Nested scanners use the same command
state and admitted context; neither an empty replacement interpreter nor a
replacement execution owner is installed to evade the borrow checker.

Full checkpoints continue to capture command and execution roots through the
existing aggregate contract. Group topology remains the sole native TeX save
stack, and mode roots remain the same `ModeNest`. Moving a field into its
execution owner changes no root, retention lifetime, capture eligibility or
rollback rule. Formats serialize semantic state, never either borrow interface.

Native semantic calls borrow the real command host alongside input and execution
state. `with_execution_owners` lends capabilities, the optional resource
provider and its matching attempted-resource slot, and the current observer.
There is no copied host, surrogate decline ledger, or observer buffer made for
that call. Semantic helpers accept that observer loan directly; the concrete
operation buffer remains only with its actual outer observation owner. Nested
scanners reborrow the same observer and host capabilities. The projection is
always inlined so these references do not create an execution handoff.

Aftergroup and afterassignment consume raw spelling into their actual native
owners. Kerns and vertical glue lend only the admitted context, mode, diagnostic
and fuel owners used by their semantic functions, then continue the same reader
loan. Their retired outer apply methods and unused command-machine construction
are deleted. These selected arms preserve the single authoritative dispatcher;
main-control fetching still returns command records until the whole switch is
moved into native consumption.

The production switch's retained processor loans the complete actual execution
owner. Mode, trace labels, box state and pointer-source bookkeeping are queried
through that owner at their consuming arm. Its mode is read again for each
command; a mode-changing semantic call cannot leave the next expansion or
switch using an earlier sample. Cold component fact providers remain only for
calls that genuinely borrow those components independently.

Horizontal glue and setlanguage consume their scanner values directly through
that loan. Their source-free helpers receive context, modes, effects and fuel;
they construct no command machine and have no executor adapter. Horizontal
glue's former internal paragraph-start branch is deleted: the sole mode switch
already owns §1090's vertical backup and paragraph entry before that arm.

## Native consumption contract

The complete native switch must run synchronously inside the canonical reader's terminal
consumer, using borrowed spelling, native meaning and control-sequence identity
as its parameters. The retained processor remains the input and fuel authority
for operand scans. The switch's semantic helper lends only actual owners;
ordinary completion continues input directly. Prefix flags belong to that
in-flight TeX operation and cannot become a driver checkpoint cut. Replacement
fetches enter the same reswitch without backup or an intervening command record.
World dialogue, prepared shipout, output-page ownership and driver checkpoint
cuts return their actual typed results after the relevant loans end.

The terminal selector and consumer adapters are inline owner projections.
Their forwarding calls must fold into the reader; extending semantic support
must not introduce per-terminal function calls or result packing at this seam.

## Native font and code-table owners

Font selection, font dimensions and character controls, generated-font
installation, and character-code tables scan and mutate through the retained
processor in the sole packed switch. Their source-free semantics borrow only
the admitted context, actual diagnostic effects, command state when needed
for recovery context, and demand-owned mutation receipts. INITEX policy is
read from the actual execution owner. They construct no `CommandMachine`,
scanner adapter, substitute host-fact record or continuation payload.

The scanner consumes its final scalar operands in that switch arm. A generated
font definition keeps its actual scanned definition value through installation;
a pattern or exception scanner transfers its actual completed words into the
hyphenation store. Ordered scanning reports, error dialogue, observer evidence
and afterassignment input retain their existing semantic seams. Loading an
external font still crosses its actual immutable-resource resolution boundary;
that boundary is not an eligibility classification for a second interpreter.

This removes the source-free adapters needed by a future native reader-owned
main switch. Production main control still retains `CurrentCommand` across
its fetch/switch boundary; the complete removal of that handoff is unfinished.

## Native shorthand and shape assignment owners

Character and register shorthands scan their provisional binding and restricted
operand in their selected switch arm, then lend the actual context, diagnostic
effects, profile state and demanded receipts to the concrete assignment. The
provisional binding retains TeX82 §1224's scope and scan order. Catcode assignment
lends those same owners and flushes pending horizontal characters through the
actual mode/fuel loan before changing the table.

Paragraph shapes and e-TeX penalty arrays scan once in that same processor. Their
completed scalar arrays are consumed by the durable assignment owner; there is
no second scanner episode, command machine or immediately consumed operand
packet. The durable owner streams the encoded words directly into its existing
publication lane, without a temporary encoded-word vector. The old penalty array
is decoded only when assignment tracing requests it, before replacement. Native
group saves and checkpoint first-touch history continue to own the installed
root through the existing eqtb mutation policy.

The scalar array scan and its durable encoding remain distinct representations.
Replacing that encoding with a native scalar-array owner, and removing the main
`CurrentCommand` handoff, remain unfinished; deleting the assignment adapters
does not establish either claim.

## Mark and output-stream operands

TeX82 §1101 and e-TeX 2.6 [26.424] append a mark in any mode without
calling the page builder. The selected mark arm scans its actual class and
balanced text through the retained processor, publishes the final token-list
owner in the admitted context, and lends only that context, modes, diagnostics,
fuel and box owner to the mark semantic routine. Pending characters flush at
that operation; restricted horizontal box migration retains its actual page
region. There is no whole-executor reconstruction or page-output capture for
this source-free operation. The replaced mark scanner and application wrappers
are deleted.

Deferred openout scans its stream, optional equals and filename in the same
selected arm before handing its final filename to the actual output-stream
semantic boundary. It opens no second processor. The replaced streams-scanner
module is deleted.

## Native group continuation

Ordinary simple and semi-simple group boundaries execute synchronously inside
main control's canonical expanded reader. Their spelling and resolved packed
meaning stay borrowed parameters; the reader performs literal brace accounting
before semantic consumption. No command record is created for an admitted group.
Group entry lends the actual context, command, effects, fuel and mode nest to
one source-free helper. Group exit uses the same synchronous unsave routine that
owns saved values and aftergroup input, with full checkpoints unchanged.

The native consumer owns an optional actual semantic completion only when a
report or failure requires returning to the driver. It never stores a token,
meaning, opcode or generic operation. Successful ordinary group consumption
increments the actual operation count and immediately continues the input loop.
Unsave checks restored tracing parameters before consuming another terminal.
Tracing, observation, character flushing, box/math/recovery group behavior and
the final driver-budget operation retain their genuine boundaries and share the
same group helpers; there is no second group engine or deferred cleanup stage.

Native terminal selection compares the exact relax word before deriving a
family/selector code. Relax consumes no operand and constructs no switch key;
only the group arms derive the native control selector they actually use.

## Scalar recovery at the consuming scan

A scalar scanner returns its actual value and the primary origin needed by
restricted-number and structured diagnostics. Missing-number and alphabetic
constant recovery already report their diagnostic and supply the canonical
zero before returning. No consumer branches on an additional recovery marker.
Integer, dimension and glue scans therefore carry no recovery enum, and nested
unit/stretch/shrink scans do not propagate one. Character-code scanning keeps
its actual optional-space decision; restricted scans retain their real range
check and diagnostic origin. Full checkpoints, input backup, observations and
error chronology continue to use the same semantic paths.

Stack conservation decides from the actual packed frame kind. It retires every
exhausted token-list kind except the two canonical v-template encodings, with
no reconstructed token-behavior enum or additional cached classification.

Relax's eligibility arm tests its actual word before checking the driver budget
and reporting state. Other native kinds evaluate their group-specific rules in
their own arm; they do not add selection work to each ordinary relax word.

Keyword and optional-equals scans return the actual boolean. Their callers use
no scalar origin, so those scans track no first-token provenance and construct
no scalar packet. Failed keyword matching still owns and backs its actual
prefix spelling; observations, alignment correction and diagnostics stay in
the same reader and backup paths.

Native group entry writes its boundary directly into the save stack and returns only success or the actual error. The caller already owns the entered kind and line; it does not receive a copied `GroupFrame`. Exit still returns the actual closed frame for tracing. The no-op relax arm returns before recording a diagnostic origin, since it produces no diagnostic or list effect.

Font selection consumes its final native font operand in the same reader through the existing assignment authority and performs the real afterassignment insertion before acquiring another word. Native no-ops, font selections and ordinary groups execute the last budgeted operation directly and return its actual result; budget exhaustion never forces a command record. The budget is admitted at reader entry and checked after each actual operation, without another per-word admission check.

Dimension and glue scans return `Scaled` and `GlueSpec` directly. Infinite dimension scans additionally return their actual glue order. These scans store no primary-origin packet and propagate no first-token provenance through signs, units or glue components, because no semantic consumer reads it. Integer, restricted-range and internal-value scans retain the primary origins their diagnostic and structured consumers need. Existing arithmetic recovery, optional spaces, raw backup, observation values and physical glue-source identity follow their original semantic positions.

Synchronous keyword, optional-equals, nested integer and expression scans return their actual value or error directly. `CommandError` keeps large payloads boxed only when raised; a success needs no separate result frame, status tag, optional error slot or extraction macro. Actual resource failures unwind expression scratch, and expression errors truncate their real stack suffix at the original error edge.

## Reader-owned scalar assignments

Count, dimension, skip and muskip registers and integer, dimension and glue
parameters execute in the canonical reader's terminal consumer. The native
meaning word selects their sole scanner/application switch; no current-command
record, delivered-token packet or assignment request is constructed for these
operations. Prefixed and observed calls enter that same switch with their actual
scope and demanded mutation receipts. Their former main-switch bodies are deleted. Parameter and shorthand targets come directly from the packed operand, without a semantic meaning decode. The glue-parameter assignment packet and scanner wrapper are deleted; its arm consumes the actual optional-equals decision and glue specification.

A scanner's actual diagnostic reports precede its mutation through the retained
context. The diagnostic helper borrows command state, execution state, context
and effects directly, and owns no processor or command. Report effects, tracing
changes, afterassignment delivery and the real operation budget stop native
continuation at their existing semantic boundaries. Operand scanning remains
inside the canonical processor and cannot add another input reader.

## Native reader code size

The native terminal adapter keeps the common no-op inline. Group, font and
assignment semantics run in one outlined substantive handler, still synchronously
inside the canonical reader. This bounds the reader's machine-code growth as
more switch families move into it; it creates no command record, operation
carrier or second interpreter. Eligibility remains inline and the actual handler
returns only its consumer control flow, with real semantic completion retained
only at a budget, diagnostic or error boundary.

## Reader-owned bindings and token lists

Definitions, expanded definitions, let/futurelet and token register/parameter
assignments use the same reader-owned assignment switch as scalar assignments.
The packed word supplies the actual selector; native scope flags and the resolved
control-sequence identity are borrowed directly. Prefixed and observed execution
call that same implementation, and their former main-switch bodies are deleted.

Token assignment scanning returns its actual collected or retained token owner.
The register target is scanned in its consuming switch arm. There are no
register/parameter assignment packets or nested right-hand-side packet, and no
pointer-presence flag or token-list read used only to derive it. A copied source
keeps its physical pointer, including a present empty list; a newly collected
empty list is null. Only a newly collected nonempty output routine receives
TeX82 section 1226's enclosing brace pair. Existing pointer-identity and output
routine tests are the semantic checks for these decisions.

Macro definition scans retain their checked target and published definition,
without an unused diagnostic provenance field. Scanner diagnostics are rendered
before the actual mutation, and mutation receipts retain their one observation
owner through a demanded diagnostic exit. Native completion inserts the actual
afterassignment token before continuing the same reader.

## Direct token-value consumption

The token-list RHS probe uses the existing canonical expanded-word consumer.
Spaces and relax words continue in that reader. Internal list values supply
actual retained owners directly. Collected text backs up the actual spelling
through the reader's shared backup seam, carrying a delivered record only when
exceptional alignment, tracing or recovery has requested it. The opening-brace
backup and absorbing transition retain their original order; ordinary probes
construct no `CurrentCommand` and add no second input loop.

## Reader-owned arithmetic

Arithmetic dispatch and target selection execute in the canonical reader.
The target selector consumes one actual expanded word through the existing
native-word scanner seam, then scans and applies its actual arithmetic operand
before returning a semantic result. The selected target and operand are the
existing typed register/value selectors. No delivered command, arithmetic
request, second input loop or operation continuation is introduced.

Native, prefixed and observed calls share this implementation. Invalid targets
materialize only their actual diagnostic spelling and meaning and report through
the existing command/effect owners. TeX82 section 1236 consumes that rejected
target; section 1269 still inserts a pending afterassignment token once. Scanned
reports precede mutation or invalid-target reporting through the same demanded
report seam as the other native assignments. The old main-switch body and
invalid-target executor adapter are deleted.

Token-value completion keeps its actual optional result until the reader
returns: an absent terminal is an input error, whereas a scanned empty braced
list is a completed null value. EOF cannot silently assign a null list.

## Raw definition targets

TeX82 section 1215's `get_r_token` consumes actual raw spelling through the
existing canonical raw-word consumer. Ordinary control sequences yield their
symbol directly; active characters use the authoritative active-character map
and intern the cell only when needed. Literal spaces are skipped by spelling,
so a control sequence whose meaning is space remains a legal target.

Malformed targets back up their actual spelling and borrowed exceptional
metadata before inserting the inaccessible target and reporting the error.
Frozen tokens retain the section 1215 consume-without-backup rule. The scanner
keeps its existing EOF retry. There is no target-projection enum, cached-identity
validation, non-space command-fetch wrapper or ordinary `CurrentCommand`.

## Direct scanner results

Scalar and structured scans return their actual value or `CommandError` through
Rust `Result`. The custom `RetainedScalarScan` union, `into_result`/`map`
projections, identity detach routine, `_retained` entry wrappers and font child
forwarder are deleted. Callers use the actual scanners directly; no aliases or
backwards-compatible scanner methods remain.

The integer, dimension, glue and filename scanners retain their existing
resource-failure unwind points, including scratch and filename lifetime rules.
Register selectors continue to apply their profile-specific restricted integer
bounds and recovery. Integer and internal-value origins remain with actual
diagnostic consumers; this change does not introduce new provenance products.

## Native terminals after text

The canonical reader consumes ordinary relax, font, group and assignment words
even when the previous command left the character loop active. It uses the
actual pending character list and current loop state, rather than a separate
eligibility cache. Relax and group exit flush that list at their existing TeX
semantic boundary; assignment scanners preserve their own operand/report/flush
ordering. No early generic flush is added.

After consuming a terminal reached from text, the reader returns its actual
semantic completion. Main control refreshes the fetch role and mode through
the same retained processor and context before reading another word. This
prevents a stale character-lookahead role without constructing a delivered
command, opening another processor or adding a successful settlement stage.
Native terminals outside text retain their existing in-reader continuation.

Bulk text boundaries feed their actual word directly to the native terminal
consumer before any command record is constructed. This call site uses a
consumer specialized for the known text boundary; the ordinary reader retains
its existing admission check outside text. No native word repeatedly probes
the mode-list owner to rediscover which reader admitted it. Both consumers
share the same substantive semantics, with the actual text completion
refreshing the reader role through the retained processor.

Ordinary spaces also enter this native consumer. Horizontal spaces append
through the existing space/glue builder; vertical and math spaces keep TeX82
section 1045's no-op semantics. A space ends character lookahead at that real
semantic boundary without constructing a command record.

## Direct binding and math-font grammars

Ordinary let and futurelet have separate scanner entry points for their actual
TeX82 section 1221 grammars. Let scans its target, optional equals and meaning;
futurelet scans its target and retains only the first token's real backup facts
across the second read. The boolean-selected scanner and futurelet forwarding
helper are removed. Each grammar returns only its final target and meaning.

Math-font family assignments scan the section 435 four-bit index, optional
equals and section 577 font selector in the selected native assignment arm.
They use the existing central font-support validation and scoped state write
from section 1234, with demanded reports before mutation and section 1269
afterassignment delivery. No second processor or semantic application wrapper
is built. The obsolete family request, its unused math-request variant and the
math-family scanner/application modules are deleted. Font identifiers referring
to a math-size bank also scan the actual restricted integer directly.

## Native entry into a character run

Horizontal letter, other-character and character-definition words enter the
existing glyph builder directly from their native meaning. They share the
same source-free scalar glyph admission as resident character runs, including
fuel, provenance, font metrics and the actual character-loop flag. Math,
paragraph-start, alignment, tracking and boundary-suppression cases retain
their real semantic consumers.

A consumed glyph returns its actual completion so the retained processor
refreshes into character lookahead before reading another word. This completion
already owns the resulting execution state: the shared continuation must not
clear its loop flag as if it were about to apply another semantic operation.
Fonts and groups leave text at their own semantic entry; assignments retain
their existing entry. No second context admission or command record is needed
to resume the horizontal character loop.

The private `HotToken` wrapper is also removed. Remaining delivered commands
hold their actual word, origin, demanded delivery site and native meaning
directly; ordinary native input constructs neither representation.

## Expansion at a character boundary

The horizontal reader consumes an ordinary macro or expandable primitive at
its actual boundary from the already resolved native meaning, then continues
its existing input back edge. Macro arguments and replacement text keep their
canonical input owners; no delivered command, replacement packet or second
expansion loop mediates the transition. Macro mismatch and paragraph/outer
recovery retain the same matching and reporting order. Undefined words,
end-template handling, observed input and exceptional delivery keep their
actual semantic consumers.

The caller statically selects whether the boundary consumer owns expansion.
Execution expands boundaries; lexical inspection returns the raw word. This
is a property of the actual consumer, with no retained eligibility flag. Both
expanded fetching and execution-owned character admission enter the same
balanced active-expansion boundary. It records quiescence once per reader
invocation and restores it at the actual return, including failure; ordinary
words add no expansion-depth bookkeeping. Fuel and raw accounting occur once
at delivery, before expansion. Source position and literal-brace accounting
remain at their original semantic positions.

## Native math characters

Ordinary math letters, other characters and character definitions append their actual noad through the retained processor, using TeX82 sections 1154--1155's existing mathcode projection, family selection and native list insertion. Active mathcodes retain the same active-character expansion and backup semantics on that processor. No delivered command or rebuilt command machine mediates an ordinary math character. Tracking, alignment, tracing and real page-owner demands retain their existing consumer boundaries.

Horizontal glyphs return their actual completion from the glyph arm because they change the next fetch role. Math glyphs keep the existing main-control role and continue its real budgeted back edge. Other native operations perform no glyph-completion classification. No cached mode, mathcode or eligibility field is added.

## Active mathcode expansion

TeX82 section 1152's synthetic active spelling is already the current token: it must not be charged or delivered again. Ordinary treatment resolves that spelling once, expands its native macro or primitive operands within the shared expanded invocation, and backs the actual terminal spelling through the existing word consumer. A nonexpandable active meaning backs its spelling directly. This avoids both the synthetic `CurrentCommand` and a delivered result created solely for backup.

Observed treatment and genuine undefined, end-template or alignment demands keep their existing delivered facts and ordering. Empty replacement, replay completion, prefix failure and paragraph/outer recovery use the same canonical macro matcher and input owners. Expansion capacity and checkpoint quiescence stay at their actual invocation boundaries; no synthetic input row or token carrier replaces either command.

## Resident frame construction

Every input level stores its actual packed frame directly beside its storage union. TokenRowHeader and the source/resident row wrappers are deleted. Common identity, source context and token behavior/retirement projections belong to the authoritative level; checkpoints copy the native frame, and source/input cursors access it directly. There is no replacement header type or compatibility accessor.

The public packed-token frame constructor is a small checked field initializer and is inlined across crate boundaries. A macro's constant frame kind, flags and zero position can then be written into its final row, rather than calling a generic constructor and copying its temporary return through additional stack locations. Frame bounds, source context, identities, retirement and peak-use accounting remain authoritative.

## Diagnostic input validity

Diagnostic coordinates project the existing next input-level identity when captured. Every real source, replacement, parameter or replay push already advances that identity, so a later push/pop cannot validate an older coordinate. No separate admission counter is stored or updated on input pushes.

The existing input context epoch changes only when source owners are rebound, checkpoints replace input, or candidate stacks exchange authority. It rejects coordinates from the prior input even when restore reinstates the same level identities and positions. Together the native identity frontier, this cold epoch and the actual top/scalar projections preserve stale-context rejection without a second hot counter.

## Math diagnostic demand

Font validation determines its actual failure once before rendering error context. Successful validation allocates no diagnostic string; a failure renders while command input is still live, publishes earlier effects at the existing barrier, and reports that same failure without rescanning fonts.

Math lowering borrows the actual command state for its synchronous error reports. The detached MathConversionErrorContext string wrapper and eager copies are deleted. Inline, display and equation-number conversion do not mutate command input during lowering; undefined-family reports render the borrowed input only at their error boundary. Display packing uses the existing live execution diagnostic context, preserving line scalars and report chronology without pre-rendering source text. No diagnostic owner survives the synchronous conversion or crosses its following input scan or group unwind.

## One authoritative input row

Each input level owns one packed frame and one InputStorage union. That union directly distinguishes a boxed source slot, an immutable macro body, a borrowed macro argument, a durable list and stack-owned token words. The outer source/resident union, separate SourceLevel and ResidentTokenRow wrappers, ResidentTokenStorage and the temporary ResidentSourceTop are deleted. No substitute row wrapper or compatibility type remains.

The frame supplies common identity, cursor, bounds, source context and retirement facts. SourceSlot owns the actual variable source state and accepts the real frame when advancing or committing its cursor. Native reading selects the storage once; it does not classify a source/resident wrapper before selecting the backing. Macro bodies retain their actual immutable definition owner and final argument region until semantic retirement.

Source and token retirement retain their existing chronology, bounds, peak usage and replay-completion owners. Full checkpoints copy the common frame, actual source slot or durable/traced words; active macro bodies and argument regions remain excluded. Diagnostics render the actual row and source slot, and native source context remains a direct top-frame projection. The change adds no per-word counter, cached source owner, separate input cursor or successful cleanup stage.
