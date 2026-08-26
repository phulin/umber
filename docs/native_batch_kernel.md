# Native Batch-Kernel Ceiling

The final deletion audit in
[`writeback/umber2-64v2.16.md`](writeback/umber2-64v2.16.md) establishes that
the measurements in this document cover the closed sixteen-control workload,
not all supported-profile semantics. The benchmark's temporary whole-job
scalar comparison adapter and runtime selector have now been deleted. Its
production-only 6M/12M/nested workload remains a performance regression gate;
external fixtures and oracle workloads remain the correctness authority.

The evaluation branch for `umber2-64v2.8`, based on
`fef7f55637842721d9ec86dc4b40c475c3a80f25`, established a production-shaped
direct-execution ceiling. Issue `umber2-64v2.10` independently reproduced that
ceiling and migrated its first bounded vertical slice into production-owned
`tex-command` and `tex-exec` code. The result is a migration seam, not a
supported runtime engine choice.

## Canonical source and input ownership

Issue `umber2-uvfm` deleted the episode's complete admitted-token program:
the root-byte copy, ahead-of-time tokenizer, `Vec<Token>`, private cursor and
backup slot, bump-owned macro bodies and arguments, and private input-frame
stack no longer exist. `NativeBatchProgram` retains only a node-capacity hint.
Each attempt constructs the production `CommandProcessor` over the live
`CommandState` and obtains commands through its ordinary expanded-delivery
entry point.

Physical roots, registered files, numbered read streams, terminal and
`\scantokens` sources, live category-code changes, source retirement, macro
and argument levels, backup and `\noexpand`, alignment templates, and root
completion consequently have one representation and one cursor. A resource
request is a typed semantic barrier rather than source coverage fallback.
`MainControl` snapshots and restores the full command/state/mode aggregate
around each attempt, retains the episode marker after a refusal, and lets one
ordinary operation advance before trying the same canonical input again. This
preserves exact rollback without transferring or rebuilding input state.

Focused tests exercise all three exact profiles, live category-code mutation,
nested registered input suspension and retry, numbered read streams,
`\noexpand` backup, alignment template levels, fragment root completion, and
post-mutation resource rollback. `CharacterProfile` and
`SourceTokenization` remain fixed telemetry counters until the final deletion
pass, but neither has a producer after this cutover and both remain zero. The
full property, command-fixture, and oracle suites exercise the production
`EngineSession` registration path, which now installs the same marker for
every root.

This cutover deliberately exposes the cost of canonical macro expansion to
the retained native dispatcher. A guarded release sample of the 1,000-call
direct workload took 36.072 ms, allocated 7,083,440 bytes in 35,185 calls, and
reported 11,328 KiB runtime peak RSS. That is a real intermediate regression
from the private-frame ceiling. Issue `umber2-3gln` owns the next refactor:
optimize the one canonical expansion/scanner/conditional/alignment machinery
instead of restoring a second fast path.

## Canonical expansion and scanner episode

Issue `umber2-3gln` moved the retained count, advance, and kern operands onto
the sole `CommandProcessor` scanner family. Optional equals, signed and radix
integers, `by`, and dimensions now use `scan_optional_equals`, `scan_integer`,
`scan_keyword`, and `scan_dimension`, including their ordinary recovery,
diagnostic, fuel, and backup behavior. Macro matching, conditional evaluation,
and alignment-template delivery already enter through the same live expanded
delivery driver. The episode owns no replacement matcher, condition stack, or
alignment frame.

Profiling then removed allocation work inside those canonical implementations
rather than adding a fast executor. Physical token delivery no longer clones a
boxed source level or allocates a temporary source-buffer projection. Frozen
control-sequence lookup hashes borrowed prefixed spellings. Macro matching
moves its packed argument words and sparse provenance roots through collection
and immutable replay instead of reconstructing them twice. Conditional
observation names remain borrowed until the runtime observer guard, so an
unobserved condition builds no record strings.

On the same host and exact 1,000-call direct work, allocation calls fell from
35,185 to 11,309 and requested bytes from 7,083,440 to 5,263,939. Five final
release processes took 41.72, 42.48, 43.29, 44.16, and 60.29 ms, with
11,136--11,328 KiB peak RSS and unchanged 67,073 fuel, artifact bytes, and DVI
bytes. The timing is host-variable and does not establish a speedup over the
earlier single 39.17 ms sample; the 67.9% allocation-call reduction is exact.

The focused production fixture now pins the honest remaining coverage result:
zero `ScannerOrExpansion` fallback and exactly four `CommandVocabulary`
fallbacks. The unsupported initial macro definition makes each attempt roll
back its prefix; ordinary execution commits the three preceding count
assignments one at a time, and a fourth refusal hands the definition itself to
ordinary execution. The closed dispatcher cannot apply that scanned
definition, or a general alignment/mode transition, while the processor
exclusively borrows the aggregate and its local whole-hbox loop owns execution.
Retrying after aggregate rollback is still exact, but relabeling the retry
would make the counter vacuous. `umber2-c1p8` therefore blocks completion of
`umber2-3gln`: it must first provide persistent canonical main-control and mode
delivery. Both issues remain open, and the legacy-deletion epic remains blocked
until the resulting real property, fixture, adversarial, and corpus evidence
reaches zero fallback.

## Independent audit and migration result

The independent audit rebuilt the original release executable from
`d3128fdf17b5ee9c158eb72ab49e8e077209b6c6`, ran the exact differential, and
then collected three fresh guarded processes per row. The direct 6M-fuel
median was 8.397 s and 225,308 KiB for production versus 133.935 ms and 47,600
KiB for the prototype: 62.7x faster with 4.73x lower peak RSS. The nested
median was 2.010 s and 55,892 KiB versus 192.839 ms and 13,144 KiB: 10.42x
faster with 4.25x lower peak RSS. The smaller direct ratio than the original
capture is ordinary host variance; exact state, artifact, bytes, DVI, effects,
terminal, and log comparison still passed.

The audit therefore confirms the claimed order of improvement under the
stated measurement contract. It does not reinterpret the direct loop as a
drop-in engine. The prototype deliberately has no per-step checkpoint,
provenance, reachability, or observation publication inside its coarse
episode. Those are barriers to batch around or materialize compactly, not work
that may be silently omitted from an observable incremental episode.

The migrated production seam is split by existing authority:

- `tex-command::NativeBatchProgram` retains only an execution-capacity hint.
  It consumes the live production `CommandProcessor` source/token stack and
  expansion machinery, then owns only the still-migrating count/group/node
  dispatcher slice.
- Production `MainControl` retains the capacity-only marker, owns its aggregate
  rollback point, projects font metrics, validates and serializes the canonical
  `PageArtifact`, commits it through the ordinary shipout transaction, and
  retains the ordinary DVI page plan.
- Typed execution refusals are the exact boundaries for an unsupported
  command family, malformed supported episode, or missing font character.
  Refusal restores the full aggregate transaction before the same
  `MainControl` continues canonical stepping. Effects, resources,
  observations, and checkpoints remain typed barriers.
- The benchmark-local lexer and kernel were deleted. The differential and
  process runner now call the production seam, preventing benchmark code from
  becoming a production dependency.

After migration, three fresh guarded samples gave a direct median of 8.055 s
and 225,284 KiB for canonical stepping versus 178.308 ms and 51,628 KiB for
the shared production episode: 45.2x faster with 4.36x lower peak RSS. The
nested medians were 1.902 s and 56,200 KiB versus 204.186 ms and 16,196 KiB:
9.32x faster with 3.47x lower peak RSS. The direct production-routed slice
therefore clears the 10x end-to-end gate. The nested reduction from the
prototype is explained by paying the canonical tokenizer during admission;
that cost remains inside the measured region rather than being hidden in
workload construction.

## Canonical count and group migration

Issue `umber2-64v2.11` removed the episode's private 256-word count table,
per-cell level table, first-local-write save vector, and group-mark vector.
`tex-state::CountGroupEpisode` addresses the live `Universe`: count reads and
writes address `Env`'s canonical fixed bank, while `\hbox`, `\begingroup`, and
their matching exits use the ordinary typed group markers and undo journal.
The scalar `Universe` assignment/group APIs and the packed episode therefore
observe one value and one restoration history. There is no synchronization
between semantic stores because there is no second semantic store.

The source/input cutover changed this object from a long-lived `Universe`
borrow into a state-free publication sidecar. Canonical delivery can now lend
the same aggregate to `CommandProcessor` between count/group operations. The
sidecar owns only a fixed dirty-count bitset and coalesces dependency and exact
identity publication; semantic values and restoration records never leave the
ordinary banks and journal.

The coarse borrow coalesces dependency and exact-identity publication only
until the next group or episode boundary. Tracked observations and observable
group/restoration tracing are typed admission barriers. The enclosing
`tex-exec` operation owns a canonical local-retry snapshot, so a semantic
barrier or artifact/DVI failure restores the count bank, group stack, journal,
dependency tracker, and state-hash roots atomically. Completed state passes
adversarial nested local/global restoration, format round-trip, exact state
hash, and snapshot rollback tests.

A fresh guarded 6M-fuel differential remained exact after the migration.
Canonical stepping took 8.226 s and 225,272 KiB peak RSS; the shared episode
took 241.339 ms and 85,612 KiB. The canonical-state path is therefore 34.1x
faster and uses 2.63x less peak RSS. Its 234 allocation calls remain five
orders of magnitude below canonical stepping's 8,423,813. The increased bytes
and peak versus the private-array ceiling are the real cost of retaining the
canonical global-assignment journal until the enclosing hbox closes, rather
than an omitted rollback obligation.

## Canonical commit and barrier protocol

Issue `umber2-64v2.15` made the production episode boundary explicit on the
same live `MainControl` and `Universe`. The existing aggregate savepoint is the
only transaction: it covers command input and replay state, mode and builder
continuations, page/PDF state, prepared DVI receipts, named boundaries,
diagnostic context, effects and artifacts, provenance watermarks, dependency
tracking, geometry observations, and private-revision allocations. Commit
freezes those canonical roots in place. Rollback restores the complete tuple
before the same scalar engine retries; no serialized transfer or second engine
state exists.

`SemanticEpisodeBarrier` distinguishes resource, effect, observer, diagnostic,
checkpoint, format, output, cancellation, fuel, and required state-identity
boundaries. `EpisodeCommit` names the primary boundary and completed operation
count. Fixed-size `EpisodeTelemetry` counts attempts, operations, commits,
rollbacks, every semantic barrier, bounded-slice returns, terminal returns,
temporary coverage boundaries, and coverage fallback by family. The counters
are operational like command fuel: rollback never refunds them and they enter
neither formats nor checkpoints.

Temporary migration debt has a separate `EpisodeCoverageFamily` vocabulary.
A coverage refusal cannot be constructed without either `MutationFreeAdmission`
or `ExactAggregateRollback`. Group- and rollback-lineage
stops after an already committed atomic action are typed coverage boundaries,
not permission to execute that action twice. Required barriers return as
barriers, never as coverage fallback. Focused perturbation tests cover each
class, exact resource/fuel rollback, observer and dependency admission,
diagnostic/effect/format publication, schema-12 fresh/load/redump equality, and
the canonical state/artifact/DVI/channel differential.

The protocol retained the no-barrier ceiling. Three fresh isolated samples per
row produced median canonical/shared times of 8.524 s/266.205 ms at 6M fuel,
16.105 s/519.695 ms at 12M fuel, and 1.981 s/211.831 ms for the nested workload:
32.0x, 31.0x, and 9.35x respectively. Exact state, artifact bytes, DVI, effects,
terminal, and log comparison passed at all three points. The shared path used
234/235 direct allocation calls and 232 nested calls.

## MainControl and EngineSession cutover

Issue `umber2-64v2.12` promoted the original admitted exact-byte family into
the real native batch lifecycle. After `umber2-uvfm`, `EngineSession` and the
CLI startup path register the root once in `CommandState` and retain only the
capacity-only `NativeBatchProgram` marker. `advance_episode` executes against
that same live input stack, `Universe`, count bank, group journal, font store,
fuel ledger, artifact ledger, DVI queue, effect world, command finalizer, and
checkpoint publisher used by scalar execution. Output returns a semantic
barrier commit; the retained session then resumes and returns the terminal
commit on its next advance.

The standalone `run_native_batch_episode` request/result/fallback API and the
benchmark `shared` engine selector are deleted. Output lowering is a private
MainControl helper with no lifecycle or state owner. Observed, diagnostic, and
tracked scalar entry points invalidate an unconsumed episode marker before
they touch command input. Aggregate execution refusals record their typed
semantic or coverage counters before ordinary dispatch resumes from the
restored canonical state.

The exact differential now also checks the operational fuel ledger. Three
fresh guarded release processes per row produced these medians:

| Workload          | Canonical | Production episode | Speedup | Canonical/episode RSS | Canonical/episode allocations |
| ----------------- | --------- | ------------------ | ------- | --------------------- | ----------------------------- |
| Direct, 6M fuel   | 8.205 s   | 250.378 ms         | 32.8x   | 225,312/76,568 KiB    | 8,423,813/5,380               |
| Direct, 12M fuel  | 16.549 s  | 526.948 ms         | 31.4x   | 421,560/141,884 KiB   | 16,841,943/5,381              |
| Nested, 1,380,089 | 1.982 s   | 209.160 ms         | 9.48x   | 56,012/23,644 KiB     | 1,845,969/5,378               |

Exact count/group state, artifact bytes, DVI, effects, terminal, log, and fuel
comparisons passed at all three points. The production episode recorded one
output barrier, one terminal commit, and zero coverage fallback for every
migrated family in focused MainControl and EngineSession tests. Schema-11 fresh
and loaded execution remains byte- and redump-identical.

## Canonical mutable node builder

Issue `umber2-64v2.13` deleted the packed episode's `NativeBatchNode` vector,
detached `PageNode` vector, private artifact construction, and private DVI
compilation. `tex-command` now receives only a borrow-scoped
`NativeBatchNodeSink`; `tex-exec` implements it with the same
`tex-state::NodeListBuilder` used by ordinary mode-list character, ligature,
kern, glue, rule, and box construction. The command loop therefore names
character and kern actions but owns no node storage.

The builder owns one mutable row stream. General and mixed material uses native
`Node` rows; an admitted character/kern-only run stays in the same builder as
an eight-byte inline row and promotes in place if mixed material arrives. At
freeze, `tex-state` derives direct-child reachability, validates handles,
computes allocation-independent semantic identity, and publishes one typed
page-arena coordinate. Mode-list semantic and TeX-physical projections use the
same builder implementation and retain only their required
projection/allocator metadata. Output, effect, observer/diagnostic, format,
state-identity, terminal, and named-checkpoint barriers materialize immutable
node sidecars before publication. Subsequent mutation invalidates them; the
existing aggregate savepoint and mode-journal lengths remain the only rollback
authority.

Packed output freezes the hbox children, constructs the canonical live
`Node::HList`, and enters the ordinary `ShipoutTransaction`. Artifact
validation/serialization, render provenance, geometry observations, effect
publication, and DVI planning now have one implementation for scalar and
packed execution.

Three fresh guarded release processes per row after the builder cutover gave
these medians. All exact state, artifact-byte, DVI, effect, terminal, log, and
fuel comparisons passed:

| Workload          | Canonical | Production episode | Speedup | Canonical/episode RSS | Canonical/episode allocations |
| ----------------- | --------- | ------------------ | ------- | --------------------- | ----------------------------- |
| Direct, 6M fuel   | 8.224 s   | 299.003 ms         | 27.5x   | 285,348/76,652 KiB    | 8,602,915/5,395               |
| Direct, 12M fuel  | 16.378 s  | 583.866 ms         | 28.1x   | 541,896/146,448 KiB   | 17,200,149/5,396              |
| Nested, 1,380,089 | 1.999 s   | 223.853 ms         | 8.93x   | 69,412/24,256 KiB     | 1,885,969/5,393               |

The rows retain 26.9%, 27.0%, and 34.9% of scalar RSS and less than 0.29% of
scalar allocation calls, so the node construction, allocation, RSS, and
absolute speed gates pass. They are 19.4%, 10.8%, and 7.0% slower than `.12`'s
production episode medians, however, so the separate five-percent stage gate
does not pass. Isolated 6M timings assign 0.137 ms to normalization, 9.512 ms
to artifact emission, about 10.36 ms to builder freeze, and 29.890 ms to
`DviPagePlan::compile_v10` reparsing the just-emitted artifact. Beads issue
`umber2-ujwo`, discovered from `.13`, owns co-emitting the canonical DVI plan
without restoring a `PageNode` store; `.13` does not conceal that remaining
output-owner gate.

## Canonical DVI plan co-emission

Issue `umber2-ujwo` closed that output-owner gate. Fresh ordinary and packed
shipout now create one operation-local `tex-out::DviPagePlanCoEmitter` beside
the artifact encoder. The completed page-arena traversal feeds
artifact bytes and page-local DVI decisions together: font registration,
characters and ligatures, kerns, glue, rules, boxes, math movement, and DVI
specials are compiled without reading the completed artifact stream. The
sidecar owns only the detached DVI plan under construction; it retains no live
node coordinate or `PageNode`, never enters artifact or format bytes, and is
moved into the ordinary prepared-DVI transaction only after artifact emission
succeeds.

This does not make DVI a second semantic authority. Canonical artifact bytes
remain the committed page representation and remain byte-identical. Loaded or
restored artifacts necessarily use their canonical bytes because no live node
borrow exists. Box and rule leaders also use the same bounded streaming-byte
adapter for that operation: TeX's leader algorithm replays a subtree at each
placement, so the adapter avoids either retaining a compatibility tree or
adding another source traversal. Every other fresh page finishes the already
co-emitted plan directly.

The repair was measured against a same-host release rebuild of `.12` commit
`faf269be4`, not against historical samples from another host epoch. Five
fresh, CPU-4-affined, order-balanced samples per revision and row produced:

| Workload          | `.12` median | Co-emitted median | Stage change | `.12`/co-emitted requested bytes |
| ----------------- | -----------: | ----------------: | -----------: | -------------------------------: |
| Direct, 6M fuel   |   258.668 ms |        263.858 ms |       +2.01% |      182,893,123 / 156,407,429 B |
| Direct, 12M fuel  |   498.207 ms |        521.703 ms |       +4.72% |      363,300,819 / 310,320,757 B |
| Nested, 1,380,089 |   197.083 ms |        198.514 ms |       +0.73% |        45,339,378 / 39,416,690 B |

All rows therefore satisfy the `.13` no-more-than-five-percent stage gate.
Direct requested bytes fell by 14.48%/14.58%. Exact comparisons passed for
state, 1,701,713/3,403,201/380,244 artifact bytes,
246,448/492,716/50,180 serialized DVI bytes, effects, terminal, log, and the
6,000,000/12,000,000/1,380,089 fuel ledgers.

## Covered semantic slice

The direct workload is a complete INITEX job. It initializes count registers,
defines a parameterized macro, and calls it inside a shipped `\hbox`. Every
call performs a local assignment, two global assignments, an `\ifnum` with
both arms exercised, character emission through a synthetic TFM font, and an
explicit `\kern`. Closing the box proves local rollback while preserving the
global values. The page is shipped and the job ends normally.

The structurally different `nested` workload defines a second parameterized
macro. Each outer call forwards its argument into two inner calls. This adds a
macro-body source level, nested argument capture, and two expansion returns
without changing the observable operation mix.

The original native kernel had one byte lexer and direct control-sequence
lookup, packed 32-bit tokens, a packed source/token cursor stack, bump-scoped
macro bodies and arguments, fixed mutable count and level arrays, and a
first-local-write save stack. The migrated slice replaces its lexer with the
canonical source tokenizer; the later count/group migration replaces all four
private count/restoration structures with `tex-state`'s direct bank and
journal. Checkpoint, provenance, reachability, and observation objects do not
cross the bounded loop; their canonical publications occur at its barriers.

The comparison meets production at canonical boundaries. Both paths return the
three count values, validated `PageArtifact`, exact serialized artifact bytes,
a successful parse of those bytes, serialized DVI, effect records, terminal
text, log text, and emitted-call count. The differential tests compare every
field byte-for-byte or structurally. Production command-work counters are the
only extra diagnostic field.

## Measurement contract

`Workload` source construction occurs before the measured region for both
implementations. The region includes fresh engine state, synthetic-font
construction and installation, complete execution, direct output construction,
artifact validation and serialization, artifact parsing, DVI lowering and
serialization, and retention of the complete result. No production-only output
is omitted. The executable black-boxes the retained result before leaving the
region.

Each sample is a fresh process of the same release binary. Times and allocator
counters come from one `stats_alloc::Region`; peak RSS is Linux `VmHWM`. Every
reported value is the median of three isolated samples. Commands use one Cargo
job, one test thread where applicable, a 600-second watchdog, and a 4 GiB RSS
ceiling. The host was an Intel Xeon E5-2650 v4 on Linux 6.8 with Rust 1.93.0.
No native pdfTeX process was run or profiled.

The two direct endpoints are calibrated exactly by production command fuel:
89,551 calls plus 10 `\relax` tokens is 6,000,000 fuel; 179,103 calls plus 26
tokens is 12,000,000 fuel.

| Workload             |     Engine | Median time |    Peak RSS | Allocations | Allocated bytes |
| -------------------- | ---------: | ----------: | ----------: | ----------: | --------------: |
| Direct, 6M fuel      | Production |     8.214 s | 225,368 KiB |   8,423,813 |   1,855,401,315 |
| Direct, 6M fuel      |      Fused |  121.497 ms |  47,600 KiB |          74 |      71,000,297 |
| Direct, 12M fuel     | Production |    16.351 s | 421,532 KiB |  16,841,943 |   3,707,077,387 |
| Direct, 12M fuel     |      Fused |  245.275 ms |  92,004 KiB |          75 |     141,967,817 |
| Nested, 20,000 calls | Production |     1.983 s |  56,008 KiB |   1,845,969 |     412,422,391 |
| Nested, 20,000 calls |      Fused |  189.926 ms |  13,144 KiB |          72 |      16,009,759 |

The direct endpoints are 67.6x and 66.7x faster, use 4.7x and 4.6x less peak
RSS, allocate about 26.1x fewer bytes, and reduce allocation calls by five
orders of magnitude. The nested scanner workload remains 10.4x faster with
4.3x less peak RSS and 25.8x fewer allocated bytes. Doubling direct command
fuel preserves the ratio, so this is steady-state scaling rather than fixed
setup amortization.

## Cycle attribution

Frame-pointer `perf` captures used the profiling Cargo profile. The final
6M-fuel production capture retained over 9,000 samples with none lost and
approximately 23.40 billion cycles. The corresponding fused capture retained
over 1,000 samples with none lost and approximately 308 million cycles. The
76.0x cycle ratio is the same order as the isolated wall-time result despite
different sampling rates and profiler perturbation.

Production's largest flat symbol was `memmove` at 17.8%. Token delivery,
control-sequence lookup, stored-token traversal, expansion, operation dispatch,
and scanners were spread across many symbols. Allocator entry points were also
distributed, consistent with 8.42 million allocations. Snapshot/checkpoint
creation and retirement, state and vector clones and drops, rooted origin and
command resolution, rollback capture, and semantic-dependency observation each
appeared separately. The production counters recorded 5.82 million raw-token
steps, 4.57 million expanded deliveries, and 1.79 million meaning lookups for
the 6M-fuel job.

The fused capture instead concentrated in `next_raw` (35.4%), `next_expanded`
(16.6%), the fused run loop (8.2%), and numeric scanning (4.3%). Artifact
parsing, traversal, construction, validation, and DVI traversal account for
over 20% in aggregate. Canonical output work is therefore paid on both sides
and is already a material share of the direct ceiling. The remaining delta is
not one removable helper: it is the multiplicative cost of representation
movement, allocation, layered delivery and lookup, snapshots, rollback roots,
and provenance/observation work around each semantic action.

## Single-engine migration

Migration must preserve one state owner, one input stack, and one dispatcher.
Temporary differential implementations are evaluation or test scaffolding and
must be removed with the layered predecessor they validate. An unsupported or
observable operation exits a direct episode through a typed barrier into the
same canonical state; it must not transfer into another live engine.

1. The packed program lives in `tex-command` and its output lowering is private
   to `tex-exec`. Its closed admission surface stops before an effect, resource,
   observation, or named checkpoint barrier. `MainControl` is the only owner
   and `EngineSession` is the only general lifecycle driver.
2. The count family now uses direct `tex-state` bank and journal storage for
   both scalar and packed execution, and its private rollback path is deleted.
   Continue the same delete-as-migrated rule when later integer families enter
   coarse episodes.
3. Replace the command machine's source/token levels internally with packed
   cursor frames and bump-scoped expansion values. The first slice already
   shares canonical source tokenization and refuses unsupported category-code
   behavior before mutation. Next, make the packed token, meaning, scanner, and
   diagnostic primitives the canonical implementations, add external-input
   barriers, and delete each superseded frame representation as its cases pass.
4. The admitted exact-byte root now enters fused delivery, expansion, scanning,
   and dispatch from the existing main-control aggregate loop. Unsupported and
   observable roots remain on canonical scalar helpers against the same mutable
   state; there is no engine selector or state transfer.
5. Give mode lists mutable native builders and construct ordinary character,
   kern, glue, rule, and box nodes directly. Freeze once at page, checkpoint,
   or externally observed boundaries, retaining `PageArtifact` and DVI as the
   canonical output seams.
6. Materialize provenance, observations, reachability summaries, and durable
   checkpoints from compact sidecars only at their named barriers. Incremental
   execution consumes those committed boundaries, not per-command persistent
   wrappers in the batch loop.
7. The benchmark-local fused lexer and executor, standalone production runner,
   and `shared` selector are removed. The retained differential compares scalar
   stepping with the real MainControl route. Continue deleting layered scalar
   families as later admission coverage replaces them.

Every stage requires exact result, diagnostic, effect, artifact, and DVI
differentials; adversarial grouping and scanner tests; 6M/12M scaling; allocation
and RSS gates; and incremental named-boundary recovery tests. Promotion proceeds
by semantic family, and a family is complete only when its old hot path is
deleted. This preserves canonical primitives while converging on one engine.

## Final canonical cutover

Issue `umber2-c1p8` completed the convergence. The capacity-only
`NativeBatchProgram`, ten-control dispatcher, count/group publication sidecar,
character/kern node sink, synthesized hbox/shipout path, packed-root marker,
coverage-fallback protocol, and packed terminal continuation were deleted.
Every retained root now enters `MainControl::advance_episode` directly.

The canonical loop commits ordinary operations directly in bounded episodes
and stops only at typed resource, effect, observer, diagnostic, checkpoint,
format, output, fuel, cancellation, state-identity, slice, or terminal
boundaries. Group entry and exit are ordinary semantic work inside an episode.
The operation body remains the complete TeX82/e-TeX/pdfTeX dispatcher and uses the sole `CommandProcessor`,
`CommandState`, `Universe`, mode nest, `NodeListBuilder`, page/PDF state,
shipout transaction, effect journal, and output ledgers. There is no command
vocabulary admission test and therefore no runtime route that can fall back to
a different semantic executor.

The production workload is now named `canonical_episode`; its old
`native_batch` name described the deleted migration kernel. Historical ceiling
measurements above remain evidence for selecting the episode architecture, not
claims about a surviving second implementation.

The final same-tree comparison used identical 10,000-call sources and output
checks with only the public driver changed between one-operation `advance` and
bounded `advance_episode`. For the direct shape, the scalar and episode medians
were 893.157 ms and 514.320 ms (1.74x); allocation calls were 615,016 and
286,436 (-53.4%), requested bytes were 194,979,672 and 81,861,980 (-58.0%),
and peak RSS medians were 40,860 and 38,996 KiB (-4.6%). For nested forwarding,
the medians were 902.073 ms and 536.304 ms (1.68x), allocation calls fell from
615,074 to 286,489 (-53.4%), requested bytes fell from 203,546,559 to
80,474,939 (-60.5%), and peak RSS fell from 39,588 to 37,916 KiB (-4.2%). Fuel,
raw steps, expanded deliveries, lookups, nodes, artifact bytes, and DVI bytes
were identical in each pair.

Profiling also found an orthogonal shared-path cost: assignment tracing
rendered escaped register names and old/new values even when
`\tracingassigns` was zero. Gating that rendering before string construction
reduced the direct episode from 406,448 to 286,436 allocation calls (-29.5%)
with essentially unchanged median CPU time. This optimization lives in the
sole assignment committer used by scalar and episode drivers; it is not an
admission shortcut.
