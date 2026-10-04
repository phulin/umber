# Minimal hot loop

Status: ownership redesign implemented; native main-control consumption remains
in progress, 2026-10-03.
Tracking: Beads `umber2-i0n5`. The selected storage and lifecycle authority is
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md).
[Main-control dispatch](main_control_dispatch.md) maps the direct interpreter.

The core reads immutable token spans through frame-owned cursors, resolves
packed meanings once, constructs macro arguments once in final retained
storage, and executes one direct mode/command/prefix switch. It uses one mutable
engine per job/checkpoint family. Independent jobs and format admissions have
independent generations. Accepted alternates and retained named roots are cold
histories, not another mutable eqtb or interpreter.

## Implemented ownership and hot work

Ordinary token delivery reads the current input frame and writes the consumer's
final native `MeaningWord`. The same native meaning value belongs to the eqtb
row and the delivered `CurrentCommand` snapshot. Resolution writes its static
word, font capability or macro flags/key directly, without constructing a
second command-word type or storing a derived class. Macro grammar, definition
collection and conditional skipping inspect those actual tags and operands;
the delivered command computes class, primitive and scalar projections only
where its consumer needs them. Dispatch projects a small, disjoint
family/selector key locally instead of branching on sparse bit-56 opcode
values; it stores no derived classification. The delivered snapshot still precedes operand
reads, so later assignments cannot change it. Font and definition capabilities
keep their complete identities, and raw recovery, alignment, tracing and
observation retain their existing settlement chronology.

The native definition admission folds into its actual activation consumer;
no separate bank-admission call copies an immediately consumed owner result.
A macro frame retains the complete immutable
words/matcher/origin payload once and then reads its slices directly. Stored
lists similarly retain their real immutable owners. Meaning reads, replacement
delivery and parameter substitution perform no checkpoint stamping, history
append or definition root count. Actual execution, scanner, recovery, tracing
and observation consumers construct the rich command they need.

Definitions have one pinned immutable format bank and one monotone counted
dynamic slot bank. Local/global scope belongs to the meaning cell's TeX level;
`\let` aliases the same key. Current cells, native group saves, checkpoint
preimages, accepted alternates, primitive meanings and newly published operands
own indexed semantic roots. At zero roots the slot tombstones without key reuse.
Active frames retain the full payload independently, so its memory charge ends
at the final actual shared-owner drop. Reads never change slot counts.

`PublishedDefinition` moves from sealing through the synchronous scanner and
executor into assignment. It survives real reporting and character-flush
errors and drops on failure. Provisional shorthand tracing renders the old
meaning before replacement. There is no namespace promotion, delayed naked old
meaning, definition frontier, accepted row tail or append-only payload history.

`ArgumentArena` owns each final argument directly: a compact range in pooled traced spelling or a sparse retained-source lane
with an immutable definition span or indexed lease on original owned words.
Singletons copy their actual traced word directly, without a tagged owner.
Contiguous spans extend without copying, and aliases flatten at capture.
Publishing transfers only the invocation coordinate; no parameter vector or
range array is copied into a separate stack. Native parameter and raw-word
counts preserve TeX capacity and high-water bookkeeping. Parameter frames use
local offsets into their actual argument value.

Delimiter holdback retains only incomplete prefixes; the completing delimiter
word is consumed directly. Brace trimming narrows the actual value. Parent
retirement, child capture, recovery backup and forwarded-source reads have
independent lifetimes. Native lease release walks its reserved free list
iteratively; abandoned capture and aggregate restore release their own actual
owners. Returned vector capacity stays with the engine and is excluded from
captures. Ordinary parameterless macros acquire no argument region or matcher
carrier. Their genuine body frame and invocation origin remain.

The expansion consumer owns the shared reader’s semantic back edge. A macro
activates directly from its resolved word and continues that same read loop;
a primitive expands there, and a terminal command returns from there. There
is no expansion-candidate result between reading and interpretation. Each
new semantic token takes one fuel charge, while refill, retirement and
parameter substitution retry under the current charge. Raw reads and spelling
collectors stop that same reader at their actual consumer boundary. The
reader invokes its actual expansion consumer inline; recursive primitive
expansion and parameter matching retain their own exceptional call frames. Each reader
clears its actual destination at its own completion or failure boundary; no
generic result-settlement adapter repeats that cleanup after the reader
returns. Main control folds its real fetch label into its retained command
consumer, but production fetching still returns `CurrentCommand` records.
Moving the complete sole switch into synchronous native consumption remains
unimplemented; the exported consumer API alone does not remove that handoff.

Assignments share one mutation policy. A local first write at a new TeX level
owns a native group preimage. A first touch in a retained host checkpoint
interval owns a separate checkpoint preimage. These histories need different
values and have different lifetimes. Repeated writes append neither history.
Parallel indexed metadata keeps levels and monotone host stamps off the
meaning-only read path. Acquire the new semantic roots before releasing the
old ones; swaps transfer existing positions without root-count churn.

Ordinary groups push one reusable native boundary. Scalar and box saves,
aftergroup words and the lazy e-TeX sparse restore marker share their actual
TeX chronology. Empty groups capture no other stores or checkpoint marks.
Unsave restores each native entry, observes it synchronously while both owners
remain valid, and then releases the losing root. Aftergroup words enter final
backup input at the native pop position. There is no restore-results vector,
second token vector or duplicated box/command/definition group topology.

The sole direct switch owns prefix accumulation, the expanded-fetch and raw
horizontal-lookahead labels, synchronous operand scanning, semantic application
and demanded evidence. Nested assignment and diagnostic entry use the same
arms. No eligibility lane, universal operation union, prepare/apply carrier,
ready mailbox, alternate dispatch or per-command rollback root remains.
There is no successful-command settlement or cleanup stage. Sealed scanner
roots are consumed by publication, insertion, rendering or splicing. Save-stack
maxima update at actual pushes, and assignments own afterassignment insertion.
See [Native command ownership](native_command_ownership.md) for the current
ownership rules; they supersede the earlier boundary-clear policy.
Specialized node/box/shipout transactions keep their real semantic obligations.

## Full aggregate checkpoint and replay

Production restart boundaries are `JobStart` and eligible main-source
`OuterParagraphEnd`. Shipout is output evidence and does not publish a restart
root. Admission requires no macro/group/scanner/build/attempt/alignment,
pending diagnostic or unfinished output owner and one empty outer vertical
mode. Open conditionals and ordinary source/stored/backup input can remain.

Capture preflights command publication and every runtime cold admission before
publishing either mark or releasing a rootless page suffix. Command and input
captures copy actual source/frame/conditional rows and transient words. Those
copies allocate and are charged; capture is not uniformly O(1), and no named
boundary deep-copies eqtb. Vacant argument pools remain engine capacity.

A paragraph cut is intent only. After the bounded driver returns from scanning,
publication, flushing and semantic application, it rechecks full quiescence to
produce eligibility. The ledger collects prepared pages; cold capture rechecks
command, mode and runtime admission before publication. Borrowed core loans
separately forbid runtime admission. Nothing can retain a restart requiring a
Rust caller or partially scanned assignment to continue.

Public restore includes the output ledger. Every component validates before
mutation. Interrupted executor, command and mode owners retire while backing
is attached; selected command/mode roots install before runtime suffix
reclamation. Selected mode captures have no node or PDF roots. Dense restore
abandons group saves without running aftergroup or ordinary unsave diagnostics.
Page/PDF/World keep their specialized root-before-payload rules; source/font
suffixes release after root installation. Output uses an infallible prepared
commit under its exclusive private-pool invariant.

A selected root survives repeated restores and each subsequent branch gets a
fresh host epoch. Candidate execution reverse-swaps accepted values into plain
cold tails on the same mutable banks. Rejection drops candidate owners and
forward-swaps accepted values; acceptance drops accepted alternates. The
oldest surviving root determines pruning. Frozen JobStart owns an exact
detached image and metadata, not a permanent decoded format engine or live
journal root at zero.

Repeatable World marks require the existing retained session mode. Eager host
publication can retire a captured effect prefix; exported sessions are sealed
against rollback. Admission rejects such roots before aggregate mutation.

A resource miss unwinds ordinary scanner, matcher, builder and Rust frames.
The host restores the full root with its matching replay prefix, admits the
verified answer through ordinary resources, and parses again. Synchronous
provider answers may complete the current call. Neither path retains a parser,
matcher, caller or output continuation. See
[Resource-miss checkpoint replay](checkpoint_resource_replay.md).

Forwarded arguments preserve source positions; generated tokens preserve their
derivation. Diagnostics detach owned text/location while their origins are
live. Output retains its actual detached recipes. `TracedTokenWord` and typed
origin coordinates remain the real storage representation; the earlier bare
`u32`/sparse-origin sketch does not supersede provenance contracts.

## Fuel, halt and scheduling units

Fuel retains the deterministic command-work charges in `tex-command/src/fuel.rs`.
One session ledger counts them across processor calls. Replay and candidate
rejection never refund spent fuel or job stack high water. Detailed census
fields compile only under `profiling`; ordinary delivery updates its fuel guard.
The earlier frame-length/line-length prepaid fuel proposal is retired.

Recoverable TeX errors report and recover synchronously at their reference
site. Fatal, resource, fuel and capacity failures propagate through ordinary
`Result` unwinding and latch the direct executor halted. A halted call cannot
resume or publish further effects; it ends the job or returns through full-root
replay. There is no deferred fake recovery value or negative shared budget
protocol. Cancellation is sampled before each driver chunk, which admits at
most 256 native operations. A single operation can perform scanner, expansion
or typesetting work; the bound is operations, not tokens or wall time. Finite
command fuel separately bounds charged scanner/expansion work. The earlier
64K cancellation-latency claim is retired; semantic scanner checks remain.

Incremental execution limits and `reexecuted_driver_chunks` count completed
bounded driver calls, not TeX commands or raw tokens. Host replay exposes
`discarded_driver_chunks`, `resource_restarts` and spent/discarded fuel. It
maintains no per-command census to preserve legacy counts. Duplicate local
retry/dispatch and always-zero savepoint projections are deleted. Native logs,
WASM wire metrics and census consumers use these actual units.

## Specialized owners and optimization evidence

The original 16-byte input frame, register-only return, universal arena/no-`Rc`
law, two-ended token heap, reference-count undo/quarantine, sparse-origin
replacement, generation-generic removal, universal cold attributes, mandatory
PGO and post-link layout are retired implementation requirements. The chosen
core preserves typed brands, real `Rc` payload owners and genuine cold owners.
They do not authorize a compatibility lane or a second machine. Layout changes
need evidence against current symbols and workloads; a frame rewrite or allocator
replacement requires an actual remaining cost, not conformity to pseudocode.

The node/PDF owner audit found no duplicate allocator or history to remove.
Page material, box roots, PDF objects and shipping transactions retain their
specialized ownership and root-before-payload contracts. A fresh full-book
profile attributes less than one percent of self samples to most individual
node/PDF ownership methods; it does not justify replacing those owners merely
to enforce a universal arena rule. Beads `umber2-i0n5.8` records that disposition.

The 64 KiB hot-set goal, PGO and symbol ordering are possible measured
optimizations, not implementation requirements or closure gates. No verified
production read/write/group/save ratio exists: the
588-page profile had detail counters disabled, and historical command counts
lack a revision-scoped census receipt. Qualitative workload reasoning must not
be reported as a measured percentage.

### Font-script ownership

TFM ligature and kern processing uses font character codes and its own
ligature program; Unicode script classification belongs to OpenType shaping.
Character admission produces a script only for an actual left-to-right shaping
font. A TFM pending run stores `Common` and performs no Unicode script lookup.
Changing between a TFM font and a shaping font already closes the pending run
because the font identities differ, independently of script compatibility.

The producer supplies the initial script to the actual pending-run owner.
Neither its constructor nor the source-span writer reclassifies the first
character. Subsequent shaping characters retain the existing supported/strong
script update and compatibility rules, including unsupported initial scripts.
The actual pending run, provenance, fuel, failure rollback and checkpoint owners
retain their existing lifetimes.

## Source ownership and validation

| Responsibility                        | Source and active proof                                                                                                                                                                                                |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Packed delivery and immutable input   | `tex-command/src/processor/expand{,/consume,/resident}.rs`, `input/{levels,stack,history}.rs`; active kernel/delivery/macro tests.                                                                                     |
| Matcher and final arguments           | `macro_definition.rs`, `definition_arena/body.rs`, `macro_call.rs`, `input/arguments.rs`; delimiter, forwarded-origin, abort, tail-reuse and zero-arity tests.                                                         |
| Native group and checkpoint histories | `tex-state/src/journal{,/groups,/checkpoints}.rs`, `env/restoration.rs`; local/global/sparse/box chronology, repeated restore, epoch exhaustion and pruning tests.                                                     |
| Counted definitions                   | `definition_arena{,/body,/build_lane}.rs`; ownership tests cover aliases, independent histories, stale keys, frame survival and final accounting.                                                                      |
| Direct execution                      | `tex-exec/src/main_control/direct/`, native semantic leaves and demanded host boundaries; observation, resource, prefix and mode tests.                                                                                |
| Aggregate restart                     | `checkpoint.rs`, `canonical_step.rs`, `Universe`, retained-generation and incremental drivers; capture rejection/exhaustion, interrupted-owner replay, repeated output restore, candidates and cold-equivalence tests. |

Every implementation deletes its displaced representation and ports real
consumers in the same logical change. No old API/format reader, shim, feature
fallback or second engine remains. Beads owns task state; documentation is not
an unexecuted compatibility checklist.

Required closure evidence is the owning semantic tests, `cargo test -q --tests`,
authoritative `scripts/check.sh`, byte-exact canonical Story, and exhaustive
canonical verification. The known canonical baseline is exactly 663 divergences
and 17 root sites, exit 1 with `VERDICT: DIVERGED`; preserve the entire ordered
worklist for semantics-preserving work. `PARTIAL` totals cannot attest equality.
See [Canonical divergence workflow](canonical_divergence_workflow.md).

Revision-scoped release receipts and profiles in Beads `umber2-i0n5.7` compare
the completed engine with the frozen pre-redesign binary. They include all 22
microbenchmarks and the unrepaired 588-page book, with exact PDF and auxiliary
hashes, fixed CPU/build/format inputs and separate sampling runs. The book,
empty groups and token-list assignments improve, while several terminal and
small-macro cases still regress against that binary. The redesign is not a
claim of uniform speedup or pdfTeX instruction parity. Historical targets below
remain informative; they do not supersede the implemented ownership contract.

## Historical measured baseline

The following measurements and source names describe the frozen `23548ce50`
full-book binary and the source read at `ea2b720e6`. They do not describe the
current implemented owners or create active storage requirements.

### Per-operation instructions

Each case runs a snippet inside a counted loop:

```tex
\def\iter{\ifnum\count255<N \advance\count255 by 1 <snippet>\expandafter\iter\fi}\iter
```

Method:

- N is 50,000 and then 250,000. The per-iteration cost is the difference in
  user instructions divided by 200,000.
- Every case except `base` then subtracts the `base` loop.
- Each run is pinned to one core.

The three engines:

| Engine        | Build                                                                            |
| ------------- | -------------------------------------------------------------------------------- |
| Umber         | The frozen `umber2-zauh` binary: optimized, frame pointers, production features  |
| Token machine | The `perf-token-machine` branch, frame-pointer release build                     |
| pdfTeX        | The clean pdfTeX 1.40.29 oracle, run as `-ini -etex`, with a minimal `texmf.cnf` |

Definitions: `\def\mzero{}`, `\def\mone#1{}`, `\def\mtwo#1#2{#2#1}`,
`\def\mdelim#1.#2\stop{}`, `\def\mbody{\relax\relax}`, and `\let\x\relax`.

| Case          | Snippet                                          |  Umber | Token machine | pdfTeX | Umber/pdfTeX |
| ------------- | ------------------------------------------------ | -----: | ------------: | -----: | -----------: |
| `base`        | the loop itself                                  | 18,949 |        15,566 |  3,814 |         5.0× |
| `relax`       | `\relax`                                         |    554 |           463 |     91 |         6.1× |
| `mzero`       | `\mzero`                                         |    356 |           245 |    216 |         1.6× |
| `mbody`       | `\mbody`                                         |  2,248 |         1,852 |    398 |         5.6× |
| `mone`        | `\mone x`                                        |  1,767 |         1,351 |    421 |         4.2× |
| `mone_brace`  | `\mone{xy}`                                      |  2,077 |         1,656 |    709 |         2.9× |
| `mtwo`        | `\mtwo\relax\relax`                              |  6,811 |         5,987 |    975 |         7.0× |
| `mdelim`      | `\mdelim a.b\stop`                               |  3,770 |         2,827 |    781 |         4.8× |
| `let`         | `\let\x\relax`                                   |  2,061 |         1,761 |    349 |         5.9× |
| `def`         | `\def\x{abc}`                                    |  4,955 |         4,196 |    866 |         5.7× |
| `edef`        | `\edef\x{\mzero abc}`                            |  6,585 |         5,481 |  1,138 |         5.8× |
| `ifx`         | `\ifx\x\relax\fi`                                |  2,179 |         1,909 |    544 |         4.0× |
| `iffalse`     | `\iffalse\relax\mzero\def\x{ab}\let\y\z\else\fi` |  3,239 |         2,750 |  1,231 |         2.6× |
| `expandafter` | `\expandafter\relax\mzero`                       |  2,615 |         2,042 |    637 |         4.1× |
| `csname`      | `\csname relax\endcsname`                        |  3,136 |         2,887 |  1,214 |         2.6× |
| `group`       | `\begingroup\endgroup`                           |  4,025 |         2,571 |    283 |        14.2× |
| `advance`     | `\advance\count2 by 1␣`                          |  6,237 |         5,319 |  1,150 |         5.4× |
| `count`       | `\count2=5␣`                                     |  5,161 |         4,316 |    876 |         5.9× |
| `toks`        | `\toks2={abc}`                                   | 17,229 |        15,592 |  1,390 |        12.4× |

`mzero` is close to pdfTeX only because an empty parameterless macro already
skips its input row.

The body frame is the real cost:

- `mbody` minus two `relax` costs is 1,140 instructions to push and retire one
  two-token body.
- pdfTeX spends 216 on the same work.

The token-machine branch's 18% base-loop reduction is the most that flattening
today's owners has achieved.

### Macro activation

`\def\mtok{\mzero}` runs 20 times per iteration for 800,000 iterations. The
profile samples user instructions every million. Self shares:

| Symbol                                         |   Self |
| ---------------------------------------------- | -----: |
| `expanded_next_hot_with_mode`                  | 27.46% |
| `activate_read_macro`                          | 16.25% |
| `push_macro_activation`                        |  8.73% |
| `admit_macro_definition`                       |  7.78% |
| `InputStack::push_row`                         |  3.94% |
| `admit_resident_macro_body`                    |  2.96% |
| `drain_depleted_input`                         |  2.91% |
| `retire_resident_ordinary_input`               |  2.35% |
| `DefinitionArena::chunk_handle`                |  2.09% |
| drop glue (`Rc` owners)                        |  1.01% |
| `classify_macro_activation`                    |  0.64% |
| loop scanning (`scan_radix_tail_from` and kin) |    ~6% |

Admission, activation, push, and retirement own about 47% of the loop. The
fused reader, resolver, and settlement own another 27%.

pdfTeX's `macro_call` (§389) does much less for a parameterless macro:

1. It saves two globals.
2. It ends exhausted token lists (§390).
3. It calls `begin_token_list`: copy the 8-field `cur_input` record into
   `input_stack`, set `start`, `loc`, and `param_start`, and add one token
   reference.

Retirement (`end_token_list`) deletes that reference and pops the record.

### Groups

There are ten `\begingroup\endgroup` pairs per iteration. Every owner below runs
for an empty group with tracing off:

| Owner and method                                                                                                         |  Self |
| ------------------------------------------------------------------------------------------------------------------------ | ----: |
| `CommandContext::roll_state_operation`                                                                                   | 4.09% |
| `CommandContext::{begin_group, end_group}`                                                                               | 5.74% |
| `CommandState::{begin_group, end_group, validate_group_payloads}`                                                        | 5.52% |
| `DurableBoxState::{begin_group, begin_unsave, begin_operation, commit_operation}`                                        | 6.52% |
| `LocalDefinitionSlots::{allocate, reclaim, lease_count, region_mut, parent}` and the arena's `begin_group`/region change | 6.68% |
| `SaveJournal::{record_group_exit_with_records, return_sparse_scratch}`, `SaveSequence::{append, truncate}`               | 4.89% |
| `leave_group_payloads`, `save_stack_words`, `trace_group_restorations`                                                   | 4.23% |
| `CommandAttempt::roll_operation`                                                                                         | 0.72% |
| libc `memcpy` and `memmove`                                                                                              | 2.88% |

pdfTeX's `new_save_level` pushes one entry and `unsave` pops to it. That is 283
instructions per pair, including both dispatches.

`trace_group_restorations` runs even with `\tracingrestores` at zero. That is
slow-tier work on the fast path.

### Token-register assignment

`\toks2={abc}` costs 17,229 instructions against 1,390.

The scanned list moves through three representations:

- the attempt arena;
- promotion: `ColdOperationPromotion::{token_root, next_token_root}` and
  `promote_resident_batch`;
- publication again: `TokenListArena::publish_words` and
  `DefinitionRegion::reserve`.

Around it runs the cold-episode protocol. Each step is a separate symbol:

- `execute_cold_episode` (4.86%), `preflight_replay_delivery` (3.18%),
  `prepare_cold_execution_episode`, `execute_scanned_cold_episode`, and
  `execute_typed_operation`;
- `observe_committed` and `settle_save_stack_usage`;
- `ModeNestStorage::commit_journal`;
- `PageRegionHistory::parts_mut`.

Heap allocation adds `malloc` (1.38%) and two `Vec::from_iter` collections
(2.2%).

### Machine-level symptoms

- `size -A` reports a 20.2 MB text section for Umber against 1.68 MB for the
  pdfTeX oracle.
- The `perf-token-machine` branch measured:
  - L1 instruction-cache misses at 65 times pdfTeX's;
  - iTLB misses at 376 times pdfTeX's;
  - a 223 KiB hot-code footprint;
  - profile-guided optimization removing 22% of instructions and 33% of cycles.
- On the full book, Umber retires 1.39 instructions per cycle; pdfTeX retires
  1.74.

Umber executes more instructions, and also executes them worse, because the hot
code does not fit in the instruction cache.

## Original performance targets

| Case      | Original | pdfTeX | Target |
| --------- | -------: | -----: | -----: |
| `base`    |   18,949 |  3,814 | ≤4,500 |
| `relax`   |      554 |     91 |   ≤100 |
| `mzero`   |      356 |    216 |   ≤150 |
| `mbody`   |    2,248 |    398 |   ≤350 |
| `mone`    |    1,767 |    421 |   ≤350 |
| `mtwo`    |    6,811 |    975 |   ≤900 |
| `mdelim`  |    3,770 |    781 |   ≤700 |
| `let`     |    2,061 |    349 |   ≤350 |
| `def`     |    4,955 |    866 |   ≤900 |
| `edef`    |    6,585 |  1,138 | ≤1,200 |
| `group`   |    4,025 |    283 |   ≤300 |
| `advance` |    6,237 |  1,150 | ≤1,200 |
| `count`   |    5,161 |    876 |   ≤900 |
| `toks`    |   17,229 |  1,390 | ≤1,500 |

Full-book targets, in order:

1. After the token machine and main-control stages:
   - at most 300G instructions, down from 644G;
   - `tex-command` and main control reduced at least fourfold.
2. After layout and node memory, within 1.5 times pdfTeX:
   - about 250G instructions;
   - about 145G cycles.
   - Instructions per cycle should reach pdfTeX's once the hot set fits in the
     instruction cache.

These are informative targets, not forecasts or implementation closure gates.

### Forward-only TFM text processing

TeX82 §§1034–1040 settle the left character before advancing to the next
pair. The native TFM engine follows that ownership: it reads the pending
source run one character at a time, retains the current left/right cells,
and uses a stack only when a ligature instruction retains or inserts a
right character. Settled glyphs and kerns go directly to the final node sink.
Character existence is queried only when false-boundary recovery actually
needs it. The former linked work graph, previous/next coordinates, detached nodes,
tombstones and full-run glyph staging are deleted.

Hyphenation's TeX82 §§915–917 physical replacement measurements require
looking at the completed main suffix while replaying an alternative. That
actual reconstitution sink requests a flat completed-output tape from the
same engine. Ordinary text and child branch sinks request no tape. This is
an output ownership requirement, not a second processing algorithm or a
font eligibility path. Nested loans preserve the parent's completed tape
and provenance and restore only the child's scratch suffix on every exit.
Fuel is still charged for each native pair transition, including retained
ligature instructions; final output remains inside the existing rollback
transaction.

The pending source vector and scratch provenance tape still duplicate source
characters. Removing that copy requires lending the actual source slice to
reconstitution callbacks; the graph deletion does not claim that ownership
change is already complete. Full main-control command-slot elimination also
remains separate unfinished work.

## Borrowed math source reads

Appendix G conversion lends immutable state for the entire kernel call. The
existing expanded math view retains each source node's actual `DirectNodeView`
borrow alongside its `(PageListId, index)` replay coordinate. A choice frame
opens its source cursor once; repeated noad inspection and adjacency reads
project the already-borrowed node instead of admitting its list and resolving
its position again. Genuinely rewritten noads still become owned drafts in the
same view. Choice expansion, styles, source retention and nested observation
replay keep their existing ordering.

The borrow is bounded by conversion and cannot survive mutation, unsave or
checkpoint restoration. There is no persistent admission cache, node registry,
new counter or alternate input owner. The existing source-node vector owns
these borrows; no parallel vector is introduced. Compact-node projection uses
the same decoder as ordinary positional reads, preserving malformed payload
handling. Public page cursors validate their coordinate once through the
existing span-reader boundary; the discarded admission before that validation
is removed. Borrowed durable-source fallback retains its existing behavior.

## Direct math dependency discovery

Dependency discovery and conversion share the selected-choice source walker.
Discovery appends actual child coordinates directly to the dependency stack;
it creates no expanded node list, style-marker vector or separate request
collection. Formulas with no child lists return before allocating a dependency
stack or inserting visiting entries.

The stack retains each parent's real postorder completion. At that point its
children are already converted, so the kernel converts it immediately and
stores its layout in the existing converted-layout map. That map also supplies
completion evidence for repeated child coordinates. A visiting set retains
cycle detection during the nonrecursive walk. The separate completed set,
request deduplication set and postorder vector are deleted. Child source order,
full cramped choice styles, delimiter style resets and per-occurrence hpack
and diagnostic replay retain their existing semantics.

Ordinary-pair processing owns its single adjacency decision in `make_ord`.
The first pass calls that processor directly after bin normalization; the
processor leaves other nodes unchanged. The preliminary ordinary-kind and
adjacency probes and their immediately consumed Boolean helper are deleted.

## Direct packing measurements

Packing consumes actual borrowed glyph coordinates for width, height and depth
measurement. Glyph traversal uses the existing `DirectNodeView` scalar
projection, so ligature provenance and whole logical nodes are not materialized
solely for measurement. Other dimension-bearing nodes match their actual
`NodeView` fields. Vertical measurement ignores glyphs before requesting a
logical projection. Leader dimensions, discretionary replacement dimensions,
image sizes, font mapping and missing-character fallback retain their existing
packing rules.

`PackedNode` and its immediately consumed conversion are deleted, together
with the full-node clone before horizontal measurement. Box and vertical
dimension accessors project their actual node variants directly. No replacement
packing enum, font cache or separate node collection is added.

## Native math operand scanning

Math fields and delimiter operands consume the existing expanded-word reader
synchronously. The scalar terminal supplies spelling and the already resolved
`MeaningWord`; ordinary classification does not initialize `CurrentCommand` or
decode a second command representation. Spaces and relax continue in that same
reader, and the scanner writes only its final math field or delimiter result.
Active math characters retain their existing expansion and restart semantics.
Braced fields still back up the exact rejected spelling and call the canonical
left-brace scanner, preserving observed delivery and missing-brace recovery.

Structural delimiter boundaries and ordinary non-radical delimiters share the
same delimiter grammar. Only a negative delimiter captures diagnostic facts
and backs up the consumed word. Observed, alignment and recovery deliveries
lend their existing exceptional record to the same consumer. Radical positions
continue to scan their restricted integer directly. The duplicate hot
nonblank/non-relax fetch loop is deleted rather than retained as an adapter.

## Native mandatory brace scanning

Every mandatory opening-brace caller uses TeX82 section 403 expanded input.
The scanner consumes native expanded words directly, skips spaces and relax
in that same reader, and returns the actual origin of an accepted brace.
An absent origin result means that section 403 inserted the brace after
reporting and backing up a rejected token; a real brace with an unknown
origin remains distinct. EOF still fails admission.

The scanner returns no command-bearing result enum and has no raw/expanded
boolean dispatcher. Its unused raw branch and the generic `ScannedLeftBrace`
carrier are deleted. Callers consume the origin or inserted-brace decision
directly. Errors prepare the report before backup, render the context after
backup, and apply the synthetic alignment contribution at the same positions.
Observed and alignment deliveries retain their existing exceptional record
only within the canonical reader, without returning it as a scanner product.

Math operand consumers also project character, character-definition, math-character and primitive operands directly from the packed meaning. The general Meaning decoder is absent from field and delimiter classification; invalid operands demand their diagnostic identity only when rejected.

## Direct node-construction dependency validation

Generated nodes initialize their final reserved slot, then validate direct
child lists from the record and its existing annex view. The existing arena
visitor computes both node and paired-annex dependency floors in one traversal.
Construction returns only those two actual scalars, the optional semantic
identity and the consumer's memory/font metadata. No child-coordinate array
or temporary dependency implementation is constructed or traversed afterward.

The constructor's inspection callback borrows the actual immutable arena and
pool alongside its final record and annex. These borrows end before reservation
completion mutates metadata. `NodeRecordDependencies` and its twelve-slot array
are deleted; no wrapper, cache or alternate publication path replaces them.
Schema decoding reports `InvalidRange`, child admission reports `InvalidRegion`,
and schema failures keep priority when both fail. Failed reservations restore
the exact node and annex suffix and prior builder root. Successful construction
retains the authoritative dependency floors for checkpoints and region moves.

## Borrowed math font-parameter sources

Appendix G parameter acquisition borrows each actual symbol and extension
font once for each math size. The pure kernel receives a statically dispatched
scalar reader over that borrow, then reads its required numbered parameters.
It does not readmit font identity or construct a generic state-cell result for
every parameter. The existing math-parameter snapshot remains the kernel's
final arithmetic input, without an additional cache or parameter array.

Mapped classic fonts retain their immutable original-TFM overrides. Unmapped
fonts read their authoritative live dimension-bank cells, including mutations,
growth, group restoration and full checkpoint restoration. The state borrow
prevents mutation while the reader is live; no source is cached across a
mutation or checkpoint boundary. Invalid or absent numbered dimensions retain
their existing zero-value policy. General dimension reads project the actual
scalar directly, while generic journal reads retain the complete typed bank
cell needed for save and restore. The replaced scalar math-parameter trait
method and command-context getter are deleted.

## Resolution returns only demanded identity

Packed-token resolution writes the actual native meaning and returns only
the resolved control-sequence identity. Active characters still resolve their
registered symbol once; frozen, parameter and non-active literal words return
no identity. The input word already owns its literal category and whether its
spelling demands meaning lookup. Reader consumers use those projections
directly, without a parallel metadata result.

`PackedMeaningResolution` and its literal/lookup fields are deleted. Raw
command constructors write their final command and identity without returning
metadata or a command/metadata tuple. Profiling records a lookup only on
branches that actually resolve the word; native collection's literal shortcut
retains its existing zero-lookup accounting. Brace handling uses the original
spelling's category rather than its effective meaning, preserving active,
frozen, suppression, alignment, observation and recovery semantics.

## Native math style heads

The four math style commands consume their packed opcode in the actual
main-control terminal consumer. In math mode they append the final
`Node::MathStyle` directly; they do not construct a command or return a
scanner request. Their native admission retains the same report, active
alignment, tracked-region and page-succession boundaries as math characters.

Observed or otherwise declined delivery keeps the actual fetched record,
then invokes the same concrete style append through the retained semantic
owner loan. Reports still precede mutation, and tracing, operation counting
and full checkpoint ownership use their existing paths. Non-math-mode
commands retain missing-dollar recovery. `MathStyleKind`, its duplicate
four-way conversion and the style variant of `MathRequest` are deleted
without a replacement carrier or compatibility entry.

## Borrowed noad decoding

An annex-backed math noad view decodes its actual fields directly from the admitted fixed
annex payload. It does not copy the 36 payload words into a temporary array,
construct a full `Node`, then project that owned node into `NodeView`.
The existing view receives only its actual `MathNoad` value. The decoder is
private to the compact codec and is also used by owned materialization, so
header, payload length, serial and field validation have one authority.
No new cache, owner, payload vocabulary or compatibility decoder is added.

## Compact scalar noads

Noads with empty scripts and an empty or character nucleus fit entirely in
an existing 32-byte resident record. The seven payload words hold the existing
three kind words and four nucleus words, including diagnostic origins. This
covers ordinary characters, text characters, operators, delimiters, accents,
and other scalar kinds without an annex allocation or payload vector.
Scripted and boxed noads use the fixed annex form; their encoder writes its
36 final words in bounded storage directly, without a heap vector.

A private header flag distinguishes the two intrinsic storage forms. Both
use one field/kind validation authority for views, owned materialization and
semantic hashing. Inline noads are leaves for dependency visitation and
region copies. They contain no child coordinate to relocate. The annex form
is reserved for values requiring it, so duplicate old scalar-annex encodings
are rejected instead of retaining compatibility. Runtime word accounting,
origin preservation, group edits, checkpoint rollback and explicit region
copies retain their existing semantic contracts.

## Native defined math symbols

A `math_given` head already carries its complete math code. The native
terminal consumer reads that packed operand and appends the final noad using
the existing `append_math_char` authority. It does not construct a command
record or a scalar scanner request. The observed dispatcher shares the same
append operation after its actual report boundary. Non-math mode retains
missing-dollar recovery, and alignment, tracked-region and pending page
boundaries use the existing declined-delivery path. The scanner's redundant
`MathCharGiven` request construction is deleted; actual numeric `mathchar`
operands still use their real scanner result. Family selection, class-7
`fam` handling, spelling origin and full checkpoint ownership are unchanged.

## Native numeric math heads

`mathchar` and `delimiter` heads select their real restricted-integer grammar
inside the native terminal handler and append the final noad through the same
code appender as defined math symbols. No `CurrentCommand` or `MathRequest`
is constructed for ordinary numeric math heads. The scanner's generic
`Character` and `Delimiter` requests and their application arms are deleted.
Numeric range recovery and scanned-operand origins remain with the actual
restricted-integer scanner; `delimiter` selects the math code with the WEB
section 1154 twelve-bit shift. Observed delivery scans its actual scalar and
publishes demanded reports before final append. Accent, radical and delimiter
boundary products remain because those different grammars consume them.

## Primitive-directed math dispatch

Remaining general math dispatch enters its actual mode block once. Completed
style, defined-symbol and numeric heads use their packed control code. Other
primitive math operands receive the actual `UnexpandablePrimitive` selected
from the static word, so the math scanner no longer receives or projects
`CurrentCommand`. The same selected primitive supplies delimiter boundaries.
Script markers use their packed category code and carry only the real
`MathScriptKind`; the immediately consumed `ScannedMathScript` and its unused
provenance are deleted. Head reporting still precedes executor-owned math
field scanning, and live group destinations and recovery ordering are unchanged.

## Direct math commands in the single mode switch

Math heads join the existing mode/packed-command switch. There is no preceding
math dispatcher, generic scanner request table or request-application switch.
Each selected arm scans only its real operands and calls its concrete semantic
routine. Operand-free noad, script, limit, choice and equation-number heads
carry their actual final kind or side, without a scanner request. Character,
delimiter, fraction, glue and dimension grammars retain their actual products.
`MathRequest`, `MathTextFieldKind`, `MathLimitKind`, `ScannedMathMuMaterial`,
`ScannedEquationNumber` and the duplicate `EquationNumberSide` are deleted,
along with their factories, mapping and universal application adapter.

The cold math boundary releases the actual processor, publishes demanded head
and scanner reports, applies the selected routine, and finishes its real host
boundary. Native completed math heads still append through their reader-owned
consumer. There is no replacement operation carrier or compatibility path.
Text-accent recovery retains its separate report-before-number scan; mathaccent
scans its real number before application. Mu glue preserves the scanner's
shared-zero identity before its loan ends. Braced fields reserve their parent
noad before field scanning, keep their actual target through the live group,
and close through the ordinary group stack. Full checkpoints preserve those
same mode lists, group frames and field destinations.

## Literal payload transfer

An admitted `TokenWord` and a static character meaning share the same low
payload: Unicode scalar shifted four bits, then category. Non-active literal
resolution transfers that validated payload directly into its final static
meaning word. It does not construct a character/category pair, validate the
scalar a second time or index the category table to immediately re-encode it.
The meaning encoder owns the payload packing and the public character encoder
uses that same authority. Active category 13 retains its actual scalar/name
lookup and live dense meaning read. Token construction and format admission
still validate scalars and category operands, including rejection of malformed
format tokens; there is no alternate representation or compatibility decoder.

## Annex bookkeeping on actual writes

Constructing an inline node does not touch the annex arena and needs no annex
rollback coordinate. The existing optional copy-operation coordinate in the
annex writer is initialized at its first actual append, then reused by subsequent
appends in that construction. On failed node construction, that actual coordinate
restores the annex suffix. A node which writes no annex data has nothing to
restore. Successful construction has no settlement, commit, or cleanup pass.
The active list's separate lifetime rollback mark remains authoritative for
aborting the complete list; full checkpoints retain their existing owner and
truncation ordering. No extra flag, transaction wrapper, or alternate constructor
is introduced.
