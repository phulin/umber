# Radical simplification review

Status: architecture proposals from a source audit on 2026-10-09, at
`3f0f5e0f1`, including the existing uncommitted command/input changes. Tracking:
Beads `umber2-w9o2`. Ten independent reviewers examined command processing,
state/storage, execution, typesetting, incrementality/observation, output,
fonts, hosts/resources, bibliography, and development infrastructure. The
coordinator checked representative findings and reconciled overlapping proposals.

This review proposes changes; it does not replace current semantic contracts.
No implementation or fresh runtime benchmark was performed. Source locations
refer to the audited checkout. Performance mechanisms below are hypotheses
unless explicitly attributed to an existing measurement.

## Recommendation

Finish the in-place flat machine, remove unused execution infrastructure, and
make immutable results shareable all the way to their consumers. Treat fewer
representations, ownership transitions, and repeated traversals as the goal.
Smaller files and more narrowly named modules are useful only when they help
achieve that goal.

The existing [book profile](book_performance_refactors.md) attributes 47.70%
of sampled cycles to command processing, 24.86% to state, and 13.57% to
execution. These are disjoint buckets from one historical diagnostic run;
they are not a current universal profile or predicted savings. The
[flat-machine prototype](radical_hot_loop.md) supplies unusually promising
directional evidence for changing those representations. It pretokenizes its
source, implements roughly a dozen primitives, and uses different build
settings from the measured production engine. Its ratios cannot be advertised
as a whole-document speedup.

The [earlier cleanup measurements](repository_simplification_performance.md)
also matter: cold executor compilation was 138.51 versus 138.65 seconds, and
the small release timing samples did not establish a stable speedup. Some
allocation counts improved substantially. Another responsibility-splitting
pass alone is unlikely to address the interpreter's structural cost.

| Direction                                 | Principal deletion                                                            | Expected benefit                              | Decision                              |
| ----------------------------------------- | ----------------------------------------------------------------------------- | --------------------------------------------- | ------------------------------------- |
| Concrete token/state machine              | Reader protocols, definition/list wrappers, argument ownership graph          | Less work per token; smaller hot code         | Finish existing `umber2-8rgx`         |
| Remove unused tracked execution           | Recorder mode, dependency clocks and projections without production consumers | Less code, state, branching and maintenance   | Strong next cleanup                   |
| Share accepted output and rendered bodies | Deep result copies and unchanged-page reconstruction                          | Lower acceptance/edit latency and memory      | Strong independent work               |
| Resource admission and catalogue reuse    | Whole-ledger staging and repeated root JSON processing                        | Less acquisition CPU and allocation           | Strong independent work               |
| Immutable fixture snapshots               | Repeated Git discovery, sibling reads and hashes                              | Faster useful test feedback                   | Strong independent work               |
| Page/layout streaming operations          | Temporary owned nodes, indexes, sorts and rewrite vectors                     | Less layout traversal and allocation          | Implement in measured stages          |
| Flat mutable node heap                    | Region-transfer geometry and topology publication for scalar edits            | Simpler mutation and potentially less copying | Design and prototype after token core |
| Sparse restart-only editor                | Suffix convergence and its identity/retention machinery                       | Much simpler incremental ownership            | Product/performance experiment        |

## 1. Command processing: one concrete machine

The production reader still has several cooperating protocols:
[ExpandedReaderConsumer](../crates/tex-command/src/processor/expand.rs#L27),
the large admission/finish contract in
[consume_input](../crates/tex-command/src/processor/expand.rs#L363),
[resident storage selection](../crates/tex-command/src/processor/expand/resident.rs#L141),
and [NativeWordConsumer](../crates/tex-command/src/processor/terminal.rs#L15).
These are largely static specializations, and resident spans amortize storage
selection. Calling them all dynamic dispatch on every token would be inaccurate.
They still multiply compiled reader bodies and make boundary ownership complex.

Complete the selected concrete machine: compact bank-addressed frames, direct
raw/expanded fetching, direct synchronous scanners, packed meanings and one
producer-maintained attention word for exceptional work. Keep generation and
host admission at the cold boundary. Delete the consumer protocols, forwarding
implementations, readiness handshakes and owning cursor adapters as their actual
clients move. Retain measured character-span operations over the same machine.
This is existing `umber2-8rgx` work, including active token-memory and group
changes; it should have one implementation owner and one completion definition.
The older build-beside migration text in `radical_hot_loop.md` is not a reason
to start another engine.

Replace the [ArgumentArena region graph](../crates/tex-command/src/input/arguments.rs#L34)
with one invocation LIFO bank and at most nine range pairs per call. A capture
pin must protect arguments while depleted parent frames retire. This deletes
retained-source leases, root counts, region release cascades and per-parameter
publication. Sequential copying may beat elaborate zero-copy capture, but
large forwarded arguments are a mandatory regression workload.

Finish one stored-list representation across macro bodies, registers, hooks,
marks and deferred writes. Delete `DefinitionRoots`, `ResidentMacroBody`,
`TokenListId` ownership and their admission bridges. Simplify
[collector destination protocols](../crates/tex-command/src/token_collector.rs#L34)
and [per-append attempt validation](../crates/tex-command/src/attempt.rs#L348)
into final-list writes or reusable synchronous scratch. Expanded collectors
need scratch where recursive expansion can interleave construction; ordinary
definitions already have a direct writer, which should survive the storage
replacement. Removing per-append validation requires replacement ownership
that prevents stale, foreign, consumed or sealed-buffer access through nested
scans and resource unwind.

The runtime mechanism is fewer owner acquisitions, pointer chains, calling
arguments, representation selections and specialized instructions. Gates must
include ordered canonical commands, live catcodes, literal-brace accounting,
alignment templates, protected/noexpand behavior, delimiter overlap, tail
calls, exact fuel cuts, diagnostics, loaded formats and aggregate replay.
Measure both macro microbenchmarks and real source-heavy documents. Ahead-of-time
source tokenization cannot replace the live catcode-dependent tokenizer.

## 2. State and storage: flatten hot data, preserve distinct lifetimes

State still combines [many typed banks](../crates/tex-state/src/env.rs#L439),
[Rc/RefCell definition roots](../crates/tex-state/src/definition_arena.rs#L62)
and [separately shared token payloads](../crates/tex-state/src/durable_arena.rs#L144).
The flat-machine change should provide the sole assignment and restoration
policy over word values and epoch/level metadata. Delete the old typed
assignment/restore switches and payload publication copies with that cutover.
Group saves and checkpoint undo must remain distinct: after a checkpoint at
`x=0`, global `x=1` and local `x=2` inside an open group need both old values
for different restores. One mutation policy can maintain both sequential
histories.

The ambitious follow-up is a generation-owned mutable node heap with compact
links, first-touch inverses for pre-checkpoint mutations, tail marks and
deferred frees. Current ownership requires
[two-lineage chunk metadata](../crates/tex-state/src/fork_arena.rs#L145),
[closure transfer/rebasing](../crates/tex-state/src/node_region.rs#L1686)
and [survivor evacuation](../crates/tex-state/src/page_node_arena/output_carrier.rs#L64).
A node heap could delete region-envelope migration, dependency-floor rebasing,
sparse interval/hole protocols and topology rebuilding for scalar edits.
Genuine shared box bodies still need ownership; TeX copies still need real
copies. This requires an explicit replacement for the physical rules in
[node_region_ownership.md](node_region_ownership.md), not an incremental patch
that leaves both systems active.

Prototype this after the token core. Measure node/link writes, undo bytes,
copied nodes, fragmentation and reclamation lag under long histories. Prove
box copy/move/unbox, marks, insertions, page succession, output retries and
candidate acceptance/rejection before choosing it. Smaller preliminary
deletions are possible: copy-only node slots are still
[cleared individually on release](../crates/tex-state/src/fork_arena.rs#L2001),
and the raw prefix has
[unconditional atomic counters](../crates/tex-dense-prefix/src/lib.rs#L24).
Removing vacancy work and compiling metrics only where demanded reduces
operations; `Option<NodeRecord>` already has the same size as `NodeRecord`.

Two concrete cold-path changes can proceed independently:

- Make the World event journal its actual owning storage. Today
  [effect_journal deep-clones seven columns](../crates/tex-state/src/world/effect_publication.rs#L31),
  then detachment clones selected effects again. Use a validated borrowed
  materialization view and movable/shared suffix storage. Preserve publication
  domains, ordinals, stream-open context and independent effect/artifact winner
  semantics. Choosing rows versus columns is secondary to eliminating copies.
- Replace hash-only owned nodes, owned font recipes and
  [formatted Debug strings](../crates/tex-state/src/universe.rs#L135)
  with an explicit borrowed semantic encoder. Emit framed fields from native
  views and token slices. Test semantic perturbations, allocation-independent
  identity, diagnostic exclusion and null/empty distinctions. Measure boundary
  allocations and time; this is not automatically a token-throughput gain.

Format loading should become a validated relocation pass into final private
storage. Current code
[decodes bincode rows](../crates/tex-state/src/format.rs#L691),
[decodes node blobs for validation](../crates/tex-state/src/format.rs#L1197),
then [decodes them again for installation](../crates/tex-state/src/format.rs#L1769)
before reconstructing durable boxes/forms. Flat validated sections or logical
columns can remove per-node envelopes, intermediate node graphs and
reconstruction copies. Reconcile `frozen_format.md` with actual encoding first;
its advertised dedicated codecs are not the current implementation. A physical
wire change needs an explicit schema/fingerprint decision. Preserve complete
untrusted-input validation and atomic publication. Measure startup, format
capture and JobStart fallback separately from steady-state compilation.

## 3. Execution and page building: operate on native ranges

Ordinary dispatch and assignments already share the canonical processor.
The remaining flat-machine integration should absorb trivial execution
wrappers and the assignment committer's policy into central writes, with cold
tracing consuming the actual committed values. This removes bridges without
creating a new semantic dispatcher. Prefix/reswitch order, afterassignment,
glue identity and tracing parameter changes remain observable.

The strongest independent execution change is one admitted streaming page
operation. The builder
[materializes every front contribution](../crates/tex-exec/src/page_builder.rs#L161),
and glue/kerns pass through replacement publication even when unchanged.
The state layer
[discards and republishes the carrier](../crates/tex-state/src/page.rs#L4417).
Fire-up then builds
[temporary insertion indexes](../crates/tex-exec/src/page_output.rs#L283),
singleton ranges and mark collections that are rescanned for each class.

Transfer unchanged contribution runs directly. Publish a replacement only
when normalization changes a node. Use the existing insertion-position
authority, reusable class slots and range builders; reduce marks once into
first/bot slots. Delete temporary tree indexes, singleton-list vectors and
unconditional decode/encode handoffs. This can use today's arena and need not
wait for a node-heap redesign. Prove terminal-kern lookahead, insertion split
remainders, holding inserts, empty mark distinctions and diagnostic ordering.
Measure page/footnote-heavy documents, descriptor publications and allocations.

Consolidate deferred write/text replay into one engine-owned replay host.
Current [shipout callbacks share a RefCell adapter](../crates/tex-exec/src/main_control/cold/pdf.rs#L688)
and repeatedly reconstruct processors. Delete the residual `CommandMachine`,
paired callback aliases and duplicate rendering conversions. Keep completed
TeX expansion material where its semantics require it, but render its native
span directly into final encoded storage. Validate deferred writes, specials,
leader suppression, diagnostics and resource-failure unwind in their actual
page transaction.

## 4. Typesetting: remove temporary topology and repeated ordering work

Use ordered line-class blocks with four fitness champion slots in the line
breaker. Current
[competitive-route retention](../crates/tex-typeset/src/linebreak/mod.rs#L1373)
scans the whole winner tail for each winner, then sorts it; merging
[sorts it again](../crates/tex-typeset/src/linebreak/mod.rs#L1472).
Accumulate each class minimum once and preserve canonical visitation order
while emitting winners. Delete repeated minima, winner sorting and avoidable
merge scratch. Tie replacement, easy-line folding, looseness, parshape and
forced-break fallback are correctness gates. Benchmark paragraphs with many
live line classes, not only ordinary prose.

An additional research option is resolving alignment spans once at their
endpoint, using grouped interval constraints and running prefix widths. This
could delete per-column span maps and repeated decrement/subtract/reinsert
propagation in [alignment.rs](../crates/tex-typeset/src/alignment.rs#L63).
Checked intermediate overflow and absent-column tabskip behavior must be proved
equivalent before replacing TeX's ordered arithmetic.

Replace the math converter's mutable flat expansion vector with a selected-choice
cursor and a small current/next rewrite window. Its
[insert/remove operations](../crates/tex-typeset/src/math/convert.rs#L552)
move suffixes during ligature and ordinary-noad rewriting. Keep the real
two-pass math algorithm and style transitions. Delete vector shifting and
temporary materialization of selected branches. Exact ligature programs,
scripts, choices, binary-noad correction and diagnostic math views need proof;
long noad sequences provide the scaling workload.

Investigate a single final-line emission pass over a line plan. The executor
currently performs separate shaping, protrusion/expansion, migration,
discretionary cleanup and repacking. Sparse diagnostic overlays may also
replace the second physical hyphenation paragraph when its information is
not requested. Preserve exact allocator statistics and both semantic/physical
diagnostics; tracing cannot merely be switched off to make this pass cheaper.
These changes need separate measurement and are less certain than the two
explicit algorithmic costs above. Keep exact TeX arithmetic and the pure,
state-free typesetting boundary.

## 5. Incrementality and provenance: delete unused modes before adding reuse

Remove the generic tracked-region execution path after checking supported
external API users. Repository production drivers
[pass no recorder](../crates/tex-exec/src/main_control.rs#L2044),
but the engine retains recorder lifecycle, poison barriers, dependency clocks,
backdating, projections and snapshot fields. Delete the tracked advance APIs,
`TrackedStepResult`, `TrackedRegionRecord` and unused recorder machinery.
Remove its tracking-enabled identity dimension coherently from the full
checkpoint root/schema: the current
[completeness check requires it](../crates/tex-exec/src/checkpoint.rs#L314),
so simply leaving that root absent would disable convergence. Retire or replace
the dedicated dependency benchmarks alongside their implementation.
Keep World external-input dependencies, pure-query caches, observations and
ordinary full-checkpoint restart. Normal execution already avoids recorder
allocation, so expect modest runtime savings and substantial maintenance
reduction. Prove cold/restart/retry/rejection equivalence and measure ordinary
instructions and text size.

Replace duplicate editor fragment metadata conditionally. Every edit
[clones the full source and rebuilds a fragment layout](../crates/tex-incr/src/lib.rs#L3180);
fragment append copies metadata through
[Arc::make_mut](../crates/tex-state/src/source_fragments.rs#L297),
and accepted histories do not call the available fragment pruning routine.
Production checkpoint remapping uses accepted bytes plus `SourceOffsetMap`;
rendered-source queries instead use detached recipes and
[offsets into the current source](../crates/tex-incr/src/lib.rs#L3854).
Fragment resolution functions are test-only, and the diagnostic Root variant
has no production producer.

One authoritative revision/source mapping can delete fragment minting,
line-expanded duplicate bytes, layout rebuilding and associated accounting.
Keep compact origins, SourceMap, detached evidence and click-to-source. First
prove length-changing edits followed by suffix convergence and queries on
reused pages. The source audit found that suffix adoption clones old detached
recipes without a source edit map, while queries interpret their offsets against
the new source without checking content identity. Command-input rehoming does
not visit these recipes. Resolve this missing correspondence before declaring
the fragment machinery redundant; no runtime reproducer was executed. The documented edit-stable and
deleted-source behavior requires an explicit contract decision and a working
replacement, rather than silently deleting it. Measure long accepted-edit
sequences, historical retention and query latency.

The radical alternative is sparse restart-only incrementality: JobStart plus
the first already eligible paragraph boundary after a page/work interval.
Delete suffix convergence, suffix rehoming and identity maintenance used solely
for that feature; retain prefix output and full aggregate restore. This could
remove a large amount of coordination, but early edits would execute the rest
of the document. Compare p50/p95 edit latency, memory and resource replay on
real LaTeX editing workloads before choosing it. Existing convergence may
remain the better product tradeoff. Parked scanner continuations and deleted
paragraph-transaction replay should stay retired.

Observation tools can independently project directly into the selected oracle
sink. For example, [Tex82Trip finalization](../crates/tex-observe/src/lib.rs#L455)
constructs semantic output that its profile discards. Remove unrequested
translation/normalization vectors and format context only when consumed.
Preserve bounded uncommitted observation prefixes and exact evidence bytes.
This improves observed runs, not ordinary unobserved compilation.

## 6. Output: share immutable results and stream redundant intermediates

Give accepted editor output one immutable snapshot shared by session, display,
stable state and completion. Publication
[clones the complete output](../crates/umber/src/virtual_compile/publication.rs#L231),
and [editor bookkeeping clones it again](../crates/umber/src/editor_session.rs#L155).
Its DVI/HTML/log/asset/auxiliary vectors can be large. Generated-file selections
should reference shared bytes or indices. Delete deep result clones and
duplicate generated-file collections. Preserve revision identity, atomic
acceptance, cancellation and provisional/stable separation. WASM still needs
its final host transfer. Measure acceptance separately from typesetting.
For retained editors, consider returning revision/output identity, metrics and
the render update, with explicit retrieval of requested DVI/HTML/log/auxiliary
artifacts. This also removes unconditional aggregate DTO conversion and byte
transfer on no-op advances and small edits. It is a deliberate public API
change; a one-shot convenience interface can materialize the aggregate once.

Reuse immutable HTML page bodies before rebuilding them. Current incremental
output
[renders every page before reusing keys](../crates/tex-out/src/html/incremental.rs#L479)
and linearly searches old pages for each result. Share bodies by artifact
identity plus relevant font/resource/options identity; keep occurrence and
revision headers separate. One old-page index and digest short-circuits can
delete unchanged-page reconstruction, hashing and diff maps. Duplicate pages,
page moves, font changes and provenance changes must invalidate or distinguish
reuse correctly. Compare full rebuilt models, patches and actual DOM identity;
prove bounded retention through alternating edits.

PDF should progress toward a staged geometry-to-painter pipeline. Current
assembly
[clones positioned pages and builds operation vectors](../crates/tex-out/src/pdf/finalize/content/assembly.rs#L70).
First borrow pages and feed the painter directly. Later let verified canonical
artifact views drive geometry, accumulating compact font/navigation/resource
summaries and private content. Delete whole-page clones, production paint
operation vectors and eventually document-wide positioned retention. Retain
the validated graph and sole object allocator. Glue rounding, leaders,
VF ordering, links, tagged spacing, snapping and object allocation order are
mandatory exact-output gates. This extends already completed one-pass lowering
and parallel output work.

Two additional bounded changes are well grounded:

- Retain one parsed source-PDF context per authenticated content identity.
  [Each selected-page inclusion reparses it](../crates/tex-out/src/pdf/import.rs#L30).
  Borrow pass-through encoded stream ranges from shared source bytes and
  preserve per-inclusion destination remapping initially. Cross-page object
  deduplication requires separate pdfTeX authority; parser reuse does not.
- Replace document-wide
  [compressed-result collection](../crates/tex-out/src/pdf/serialize.rs#L101)
  with a byte-budgeted ordered work window. Release compressed streams as they
  serialize. Preserve zlib inputs, object order and deterministic first error.
  The graph and final PDF still occupy memory; total memory is not constant.

Keep detached canonical artifacts, independent DVI lowering, output-resource
closure and atomic publication. Direct live-node-to-final-PDF output would
discard valuable boundaries rather than remove redundant intermediates.

## 7. Fonts and Unicode: retain one parsed program and derive once

Generated-font identity lookup is already indexed. The remaining miss path
[clones metrics before transforming them](../crates/tex-fonts/src/metrics.rs#L1047)
and [clones tables again during expansion](../crates/tex-fonts/src/metrics.rs#L1422).
Construct transformed tables once and share unchanged recipes/programs.
Keep direct width tables and mutable per-font parameters; replacing every hot
width lookup with a lazy transformation would undermine the improvement.
Preserve current fontdimens when copying, letterspacing's source-parameter and
zero-quad rules, and TeX font-memory capacity accounting. Measure
expansion-generated fonts separately from existing-font lookup hits.

Publish one retained local-instance graph for virtual fonts. The host
[loads a sized TFM before its cache check](../crates/umber/src/pdf_output/finalization_input/virtual_fonts.rs#L280),
while [detached VF lowering reparses and clones metrics](../crates/tex-out/src/pdf/vf.rs#L370).
Resolve immutable program edges and sized instances once, then have adapters
and finalizers consume a sealed graph keyed by exact TFM content, size and
inherited expansion. Delete repeated parse/hash/scale work and instance
realization. Resource reservation and packet painting retain their distinct
responsibilities. Preserve declaration and first-use numbering, default-local
font selection, unpainted definitions, packet bounds, cycles and exact DVI/PDF
behavior.

For Unicode mode, separate a validated OpenType program from shaping request
options. [parse_reusing](../crates/tex-fonts/src/opentype/parse.rs#L304)
shares backing bytes but repeats validation and metadata projection. Reuse the
validated owner for feature/script/language changes; remove unused copied
GDEF/GSUB/GPOS byte owners if the cached face is their real consumer. HTML math
glyph queries should use that same owner instead of reopening font faces per
glyph. Share metrics and cached-face projections only for the same face and
resolved variation coordinates, preserving named-instance identity and each
caller's limits. Transport, program, instance, realized-font and PDF-resource
identities remain distinct equivalence relations. Retain exact classic TFM/PK
behavior and Unicode shaping as distinct semantic contracts. Test
variation/feature keys, malformed fonts, fallback, glyph metrics and
repeated-session memory before claiming savings.

The ambitious Unicode follow-up is a native run/cluster node carrying source
spans and authoritative shaped advances. Today's
[OpenType hmode path](../crates/tex-exec/src/box_runtime/hmode.rs#L979)
converts advances into nominal characters plus synthetic adjustment kerns;
final-line reshaping removes them and reconstructs the source. A native run
could delete those conversions, arrays and reconstruction walks. Keep both
necessary shaping passes and classic TFM lig/kern semantics. This needs an
artifact-schema decision and proof of discretionary boundaries, diagnostics,
provenance and packing geometry; measure Unicode workloads separately. Bidi
remains deferred.

## 8. Hosts, resources and browser: admit deltas and reuse parsed catalogues

Use a prepared resource-admission delta instead of cloning the complete
resource state for rollback. Current
[VirtualResourceAdmission](../crates/umber/src/virtual_compile.rs#L837)
clones workspace/lifecycle/key trees; nested project orchestration stages
another copy. Binding limits also repeatedly scan the ledger. Validate a
batch against an immutable base, reserve its actual additions, then commit
the prepared delta under one authority. Delete whole-ledger staging and
recounting; maintain the binding count at real transitions. File bytes already
use SharedBytes, so this is principally metadata and selected font/PK payload
copy removal. Retained snapshots may still require copy-on-write storage.
Prove mixed valid/invalid batch atomicity, duplicates, negative outcomes,
limits, cancellation and native/WASM equivalence.

Make the browser catalogue session use its retained parsed root directly.
Despite retaining it, both
[prepareBatch](../crates/umber-wasm/src/catalog_boundary.rs#L49)
and [planBatch](../crates/umber-wasm/src/catalog_boundary.rs#L102)
call the JSON-taking parser, and preparation serializes the whole root back
to JS. Delete `root_json` from ordinary lookup and return shard needs only;
send initialization metadata once. This removes repeated parse/serialize and
boundary transfer without changing resolution semantics.

Route acquisition digest verification through the required WASM binding.
The browser has a
[per-byte BigInt hash implementation](../crates/umber-wasm/js/manifest-resolver.js#L1238),
and shards are hashed again on Rust admission. Retain a verifier for standalone
HTML preview and preserve trust-boundary checks; delete redundant acquisition
passes and duplicated hash policy. Bound resolver payload caches by bytes and
release JS shard copies after successful Rust admission where retry ownership
allows it. Measure cold/warm resource discovery, boundary bytes and peak JS/WASM
memory. Keep host-owned full-checkpoint replay and improve readiness rather
than rebuilding scanner continuations.

## 9. Bibliography: indexed documents and parsing only on cache misses

Classic execution
[parses all datasources before its prepared-database lookup](../crates/bib-engine/src/classic_execution.rs#L112).
The cache key's path/content/options facts are already available. Move lookup
before parsing; misses alone build raw databases. Use source-backed spans for
raw text and a special identity offset mapping for UTF-8 instead of the
[usize entry per input byte](../crates/bib-input/src/bibtex/raw.rs#L343).
Delete parse-on-hit vectors, duplicate spelling and identity-offset arrays.
Prove parser calls are zero on cache hits and preserve malformed-input recovery,
decoding offsets, source order and cache limit enforcement.

Use an indexed Biber document arena with ordered section/list overlays and a
consuming freeze. Current list construction/validation and
[entry lookup](../crates/bib-model/src/document.rs#L289)
perform repeated linear searches, while inheritance and freezing clone entry
and field values. Resolve keys once to typed slots, share immutable base values
and move completed fields into the published owner. Delete per-section full
entry copies, cloned memo entries and list-member string searches. Preserve
observable ordering, section-specific inheritance and provenance. Measure
1k/2k/4k-entry scaling with many lists and shared parents.

The classic VM can use borrowed/shared immutable strings and contiguous entry
variable slabs. Current
[VmValue owns Strings](../crates/bib-engine/src/classic_style/vm.rs#L18),
so literal/field reads, duplication and sort-key collection copy text. Delete
read-only string clones and per-entry variable allocations; retain compatibility
visible pool accounting and bounded call/loop execution. Format serializers
can likewise emit into one bounded encoded buffer, deleting whole-output
UTF-8/newline/encoding copies in
[OutputSink](../crates/bib-output/src/router.rs#L209).

Correct the compatibility basis before prioritizing a larger Biber execution
plan. `bib.md` claims pinned compatibility is complete, but
[production explicitly leaves sourcemaps/data-model semantics inactive](../crates/bib-engine/src/biber/mod.rs#L89).
The 1,275-row manifest is inventory; a reviewed six-cohort subset has 346 ignored
rows among 417. This is not an expanded whole-suite failure count. A single
compiled control plan could replace parallel partial models and repeated
label/sort transformations, but sorting improvements are prospective until
that plan is actually exercised. Preserve separate classic and Biber semantics;
normalizing classic through Biber would lose compatibility-visible spelling,
recovery and accounting.

## 10. Development infrastructure: make evidence cheap and truthful

Earlier private source-shape guards were already replaced and dormant executor
evidence was reactivated. The retained inactive-source audit has no exceptions.
The proposals here concern repeated loading, build dependencies and execution
ownership, rather than reopening that completed cleanup.

Load committed fixtures into one immutable validated case snapshot.
[FixtureCase::read/path](../crates/test-support/src/closed_case.rs#L348)
revalidate the entire case, including hashed siblings; individual
[payload access rediscovers Git closure](../crates/test-support/src/git_fixture.rs#L133)
with three Git processes. Reading R payloads with H hashed siblings can repeat
O(RH) work. Validate membership, ancestry, hashes and closure while loading
bytes once; semantic tests subsequently borrow them. Delete repeated validation
facades. Keep live revalidation for mutable publication/path consumers and
preserve symlink/changed-fixture negative controls. Count Git launches and
hashed bytes before comparing suite time.

Consolidate the six optional benchmark workspace roots into one optional
benchmark workspace/lockfile/checkout-local target. Retain separate packages,
workloads and intended feature resolutions; the timing package that excludes
profiling must keep that property. Delete nested workspace/profile/lockfile
duplication. Preserve fuzz and experimental prototype isolation. Measure cold
and warm builds, and compile every retained benchmark so API drift cannot hide
behind dormant scripts.

Split heavy PDF assertion support out of lightweight test-support.
[test-support unconditionally depends on Hayro/pdf-writer](../crates/test-support/Cargo.toml#L9),
which focused command/oracle/reference tests do not need. A dedicated host-only
PDF support package can remove that dependency/invalidation subtree from those
builds while preserving an independent PDF query model. Full-workspace builds
still need it; focused feedback is the principal benefit.

Reuse the existing receipt-based corpus executor for older LaTeX lanes. Their
shell runners separately own build-on-run, watchdog, staging and receipt logic;
the arXiv runner already has explicit prebuilt inputs, identity, resume and
bounded execution. Delete duplicated execution machinery, retaining small
lane-specific adapters and each pinned TeX Live authority. Preserve exact
eligibility, recorder closure, multipass files and comparison rules.

Keep one concise current contract per owner, label proposals explicitly, and
retain historical decisions/receipts in Git and Beads. Replace remaining
catalogue test-name source scans by extending the existing compiled-test
discovery mechanism to catalogue-linked tests; it currently covers ignored
host tests outside bibliography. Reuse its prebuild artifact listing. Report
declared, discovered, selected and executed evidence separately. Discovery is
not proof of execution. Avoid adding a documentation-inventory framework or
more layers around gates to perform this cleanup.

## Delivery discipline

Concentrate core implementation on the existing flat-machine convergence.
Resource/fixture work and immutable-output ownership are suitable independent
changes because they have clear deletion boundaries. Defer a broad node rewrite
until its profile and focused prototype justify it. Treat restart-only editing
as a measured product alternative, not an automatic simplification win.

Each implementation should state which representation, path or owner it
removes; preserve external semantics with the smallest independent evidence
that covers the change; and compare complete work at pinned revisions. Measure
instructions/code size for the interpreter, retained bytes/copies for session
and output changes, scaling slopes for algorithms, and build/test latency for
infrastructure. Allocation-count reductions alone do not establish faster jobs.

After focused implementation evidence, run the routine native suite and the
authoritative `scripts/check.sh`, with browser/external/corpus lanes selected
where their contracts apply. This source review changes documentation only;
its verification is formatting and source-link checking. No benchmark gains
or new runtime correctness results are claimed.
