# Canonical Engine Architecture Decision

Status: selected direction for Beads issue `umber2-64v2.14`; implementation is
ordered below and is not present merely because a prototype branch exists.

This decision selects one canonical TeX engine whose ordinary unit of work is
a bounded mutable semantic episode. It rejects both a permanent batch engine
beside the incremental engine and continued scalar object publication around
every token. `EngineSession`, `MainControl`, `CommandState`, and `Universe`
remain the sole lifecycle, dispatch, input, and state authorities. Packed
cursors, direct banks, mutable builders, and compact diagnostic sidecars live
inside those authorities and freeze at typed barriers.

The decision is based on the committed prototype branches and their benchmark
programs, not on ratios copied between unlike workloads. No native TeX-family
process was run or profiled for this comparison.

## Evidence and comparability

The rows below report the strongest result each prototype actually measured.
They are not all estimates of the same quantity. “Production” means an exact
6M/12M command-fuel run of the pinned document workload. “Slice” means the
complete but closed INITEX vocabulary in the native-batch benchmark, calibrated
to production fuel. “Focused” means a synthetic subsystem operation count.
Current RSS and peak RSS are not interchangeable, and allocator regions with
setup excluded are not interchangeable with complete-job allocator regions.
An unreported cell remains unreported rather than being inferred from logical
payload bytes.

| Issue                             | Fair comparison boundary                                                                                | CPU result                                                                                                                              | Allocation result                                                                           | RSS result                                                              | Decision                                                                                                        |
| --------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `.1` compact token VM             | Focused 6M/12M delivered words; setup excluded; current RSS                                             | Borrowed fixed cursor -20.5%/-10.4%; snapshot-safe indexed cursor +5.6%/+27.7%                                                          | Hot-loop calls and bytes -100% from 187,500/51 MB and 375,000/102 MB                        | 3.0–3.2 MiB noise, no ordering                                          | Reject the owned VM; reuse packed slice cursors only inside a borrow-scoped episode                             |
| `.2` coarse regions               | Focused resolutions, 10k revisions, and 1M live values; production Amdahl bound from a separate capture | Handle resolution -63.1%/-63.2%; 256-operation cursor -23.2%/-14.9%; at most about 1.4% whole-run                                       | Accepted calls -89.3%, bytes -40.8%; rejected calls -97.6%, bytes -88.6%                    | Focused live-value RSS -70.1%                                           | Defer a general region store; reuse coarse episode lifetime and resolve-once discipline                         |
| `.3` delta checkpoints            | Focused 16K-name fork and production snapshot timer                                                     | Snapshot capture is only 0.49%/0.50% of 6M/12M CPU                                                                                      | Fork 18,991 calls/4,229,561 B to 59/86,447 B (-97.96% bytes); 10k rejected forks retain 0 B | Process RSS not reported; one fork peaks at 86,116 requested-live bytes | Adopt sealed bases, overlays, and marks for incremental state, not as a batch-speed claim                       |
| `.4` run provenance               | Exact production 6M/12M work; no-profiling release comparison                                           | +1.18%/+8.37% cycles; +1.91%/+9.35% instructions                                                                                        | Logical origin payload -73.80%/-71.88%; allocation-call census not reported                 | -0.68%/-0.48% peak RSS                                                  | Reject scalar range resolution/materialization; permit an episode sidecar only after a new producer-level gate  |
| `.5` packed hot state             | Focused six-table lookup plus exact production 6M/12M                                                   | Focused cycles -77.6%/-77.1%; production cycles -6.38%/-2.16%, with 12M cache misses +15.2%                                             | Focused setup calls -90.1%, bytes -83.1%, retained bytes -81.5%                             | Focused about -70%; production -1.11%/-0.91%                            | Reject the parallel six-table engine; adopt direct banks and validate handles at episode/publication boundaries |
| `.6` region/cursor combination    | Executed as `.2`'s once-per-256-operation row, not an independent branch                                | Local -23.2%/-14.9%; whole-run bound about 1.4%                                                                                         | Zero hot-loop cursor allocation                                                             | No combined production RSS row                                          | Close as subsumed; carry its bounded-borrow invariant into episodes                                             |
| `.7` capability ablation          | Exact production 6M/12M with pinned inputs and work vectors                                             | Decline checkpoints -3.0%/-3.2%; render-source off -3.3%/-3.4%; unshippable erased provenance -7.1%/-7.6%; no combination approaches 2x | No-checkpoint 6M calls -0.864%, bytes -2.831%                                               | No checkpoints -3.8%/-5.6%; erased provenance -4.3%/-6.2%               | Reject a second capability-stripped engine; retain typed inactive policies and real semantic barriers           |
| `.8` fused kernel                 | Complete closed INITEX slice; fresh process, complete output retained                                   | Direct 67.6x/66.7x at 6M/12M; nested 10.4x                                                                                              | Direct 8,423,813/16,841,943 calls to 74/75; allocated bytes about 26.1x lower               | Direct 4.7x/4.6x lower; nested 4.3x lower                               | Accept the fused-episode architecture, not the benchmark implementation                                         |
| `.10` production seam             | Same slice, now using production tokenizer/output and typed pre-mutation fallback                       | Direct 45.2x; nested 9.32x                                                                                                              | Shared-seam allocator totals not recorded in its decision document                          | Direct 4.36x lower; nested 3.47x lower                                  | Accept as the first migration seam, conditional on eliminating duplicate state                                  |
| `.11` canonical count/group state | Provisional fresh guarded 6M slice at branch `0f1ea9e45`; issue still open                              | 34.1x                                                                                                                                   | 234 calls versus 8,423,813; byte total not recorded                                         | 2.63x lower                                                             | Direction accepted; promotion waits for closed Bead, 12M/nested medians, and all gates below                    |

The focused rows are mechanism evidence. They cannot be multiplied or added to
predict an end-to-end result: they exclude different setup, output, storage,
and lifetime work, use different RSS definitions, and frequently time a much
smaller operation set. The production rows in `.4`, `.5`, and `.7` are directly
comparable to their own baseline because their complete work vectors match.
The `.8–.11` rows are directly comparable only within the closed native-batch
workload. That workload pays engine creation, font setup, execution, artifact
validation/serialization/reparse, DVI, effects, terminal/log output, and result
retention on both sides, but it does not exercise arbitrary category-code
changes, external input, resource suspension, observation, diagnostics, or
named incremental checkpoints inside an episode.

The `.8` result is nevertheless architectural rather than a benchmark trick.
Doubling calibrated fuel retained the direct ratio, a structurally different
nested macro/scanner shape retained a 10.4x result, `.10` independently rebuilt
and reproduced the order of magnitude, and moving tokenization plus output into
production-owned code retained 45.2x. `.11` then replaced private count and
group arrays with canonical journaled state and retained a provisional 34.1x.
The ratios may fall with broader vocabulary, but the qualitative result has
survived each fairness obligation added so far.

## Why local wins did not compose

The scalar engine pays several small representation taxes around one semantic
action: acquire an owner, resolve a handle, reconstruct a rooted token, build a
`CurrentCommand`, cross delivery and scanner layers, journal or snapshot the
aggregate, publish identity/dependency/provenance state, move buffers, then
drop the temporary ownership graph. A focused prototype removes one tax but
leaves the action passing through every other boundary.

That is why the local results obey Amdahl's law:

- the winning region cursor accelerates a production delivery attribution of
  only 9.27%, bounding its whole-run contribution near 1.4%;
- step snapshots are already batched and account for about 0.5% of CPU;
- packed lookup removes roughly half of lookup-family cost, but lookup is no
  longer the dominant cost after the cutover, so whole-run improvement falls
  to 2–6%;
- provenance payload bytes are not simultaneously live RSS, and resolving a
  range for scalar delivery adds enough instructions to lose CPU; and
- optional checkpoint, observation, source-retention, and identity policies
  are already selected at coarse boundaries, so deleting them cannot remove
  the required command/state/reachability work.

The fused kernel changes the amortization unit. It retains packed cursor and
macro state across many logically visible commands; scans and dispatches
without constructing the scalar adapter graph between each step; mutates a
direct bank and save journal in place; allocates macro bodies, arguments, and
nodes in episode-scoped builders; and freezes state only at a boundary that an
external consumer can observe. This removes repeated representation movement
and allocator/ownership traffic together. Its gain is multiplicative because
one coarse lifetime makes the packed cursor, direct state, scratch allocation,
and fused dispatch useful at the same time.

## Selected canonical engine

The end state has one engine, not “batch” and “incremental” engines:

1. `EngineSession` remains the only host lifecycle. Resource retry, effect
   publication, cancellation, and incremental revision ownership stay outside
   the semantic hot loop.
2. `MainControl` remains the only dispatcher. Its ordinary implementation runs
   a bounded semantic episode over the same live `CommandState`, `Universe`,
   mode nest, page state, and output ledger used by a slow command.
3. `Universe` owns one dense canonical environment backing plus one typed group
   and rollback journal. Existing APIs become accessors over that backing; an
   episode never mirrors or synchronizes semantic cells.
4. The command core owns one packed source/token-frame representation. An
   owned checkpoint stores stable coordinates; one processor episode validates
   them and borrows direct slices for its bounded lifetime. No per-word weak
   upgrade or generation check enters the loop.
5. Macro bodies and arguments use append/bump scratch within an episode and
   freeze into the canonical immutable form only when future engine state must
   own them.
6. Mode construction uses mutable native builders for ordinary characters,
   kerns, glue, rules, and boxes. A page, checkpoint, observation, diagnostic,
   or effect barrier freezes canonical immutable node payloads and their typed
   sidecars.
7. Provenance, reachability, dependency observations, exact state identity,
   semantic observations, and TeX-memory projections retain their current
   semantics. Compact episode sidecars collect the minimum exact information
   needed to publish them at a barrier. They are absent only when the existing
   typed policy says the consumer is absent.
8. Formats remain validated handle-free schema-12 semantic images. A loaded
   immutable base and its mutable overlay use the same banks and publication
   rules as INITEX. Runtime cursors, builder capacity, generations, journals,
   provenance, and caches never enter format bytes.
9. Incremental execution continues only from executor-named quiescent
   boundaries. At such a boundary all transient scanners and builders are
   drained, semantic state and exact sidecars are frozen, and the existing
   checkpoint tuple is captured. Incremental equality remains cold Umber
   equality, which remains canonical-reference equality in the shared domain.

“Fused” describes an internal loop, not an externally coarser TeX semantics.
Fuel is charged per canonical logical action. With an observer attached, the
episode decomposes into the identical ordered canonical events. Diagnostics,
effects, and resource requests retain their exact ordering.

## Retained, deferred, and rejected substrate

| Prototype substrate                                      | Disposition in the selected engine                                                                                                                                               |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Packed token words, cursor frames, slice arguments       | Retain and make canonical inside `tex-command`; do not retain `.1`'s owned indexed VM beside `CommandState`                                                                      |
| Coarse value regions                                     | Defer a general store migration until an accepted/rejected revision workload shows a whole-session need; use ordinary Rust borrows and episode scratch for the hot path          |
| Sealed immutable generation base plus overlay/tombstones | Retain for accepted incremental substrates and format-backed state; do not require cold batch to fork it                                                                         |
| Run-length provenance                                    | Reject `.4`'s scalar materialization design; range/repeated encoding may be reconsidered only as an append-only episode sidecar with neutral producer CPU and exact query parity |
| Dense banks and direct generation handles                | Retain dense canonical banks; validate handles at admission/publication boundaries; reject parallel token/macro/glue/node tables and repeated hot-loop validation                |
| Capability-stripped batch mode                           | Reject. Existing typed policies already make absent observers/checkpoints cheap, while demanded capabilities are semantic obligations                                            |
| Fused mutable semantic episode                           | Adopt as the ordinary internal execution shape                                                                                                                                   |
| Private batch count/group arrays                         | Delete. `.11`'s single canonical bank/journal invariant is a prerequisite for promotion                                                                                          |
| Benchmark lexer/kernel and whole-job comparison adapter  | Retain only as temporary differential evidence; delete after production coverage and gates make them redundant                                                                   |

## Barriers, fallback, and deletion

A _semantic barrier_ is required by the engine contract. A _coverage fallback_
is temporary migration debt. They must not share one untyped “slow path.”

Typed semantic barriers include:

- `NeedResource` or host input suspension;
- irreversible effect publication and readable same-run output;
- an attached semantic observer;
- a diagnostic/recovery point that needs exact input and expansion context;
- executor-named `JobStart`, `OuterParagraphEnd`, or `ShipoutComplete`
  checkpoint publication;
- format dump/load publication;
- page/artifact/DVI commit; and
- cancellation, fuel exhaustion, or a required state-identity observation.

An episode reaches a barrier by finishing the current canonical atomic action,
freezing every future-relevant builder, publishing ordered sidecars and
mutation receipts, and returning a typed result to the same `MainControl` and
`EngineSession`. A resource or recoverable failure rolls the same aggregate
back to its local-retry mark. No barrier serializes state into a second engine.

The implemented boundary is `tex-exec`'s `EpisodeCommit`,
`EpisodeCommitBoundary`, `SemanticEpisodeBarrier`, and `EpisodeTelemetry`.
Main control's existing aggregate savepoint is the sole rollback authority and
includes command, mode/builder, page/PDF/DVI, effect/artifact,
provenance/dependency/observation, diagnostic, checkpoint-schedule, and private
allocation roots. `EpisodeCoverageFallback` separately carries its temporary
semantic family and a mechanically checked mutation-free-admission or exact-
aggregate-rollback proof. These protocol values are operational evidence, not
format or checkpoint state.

A coverage fallback is allowed only when its enum variant names an unmigrated
semantic family and one of these mechanically checked conditions holds:

1. admission has made no semantic mutation, allocation publication, effect, or
   observation; or
2. the aggregate transaction restores the exact pre-episode command, state,
   mode, page, provenance, dependency, effect, and output roots before scalar
   execution resumes.

Every fallback variant must have a counter and an exact differential test. It
may call the current scalar helper against the same live engine, but it may not
select a second lexer, input stack, state store, dispatcher, or engine kind.
Fallback is deleted family by family when the canonical property catalogue,
command fixtures, focused adversarial cases, and selected corpus execute that
family with zero fallback entries. At that point the old representation and
adapter are deleted in the same change. An externally required barrier remains;
an implementation-only group or vocabulary barrier must disappear once shared
state makes it unnecessary.

Convergence is complete only when:

- no public or internal runtime engine selector exists;
- production contains one source tokenizer, input stack, expansion/scanner
  implementation, count/register backing, group journal, and node builder;
- `NativeBatchFallback` has no coverage-gap variants on supported profiles;
- the whole-job comparison adapter and benchmark-local semantic executor are
  gone;
- no migrated scalar frame, owner sidecar, rollback path, or persistent list
  builder remains reachable from production; and
- retained tests target the canonical engine directly, with differential
  scaffolding limited to external fixtures and versioned artifacts.

## Migration order

The work is deliberately serial at ownership cutovers:

1. Finish `.11`, then promote `.10` and `.11` together. Count and group state
   must have one backing before a general entry path can depend on the episode.
   `.11` is not complete until its Bead is closed and its provisional result is
   replaced by the full gate below.
2. Execute `.15`: establish the typed episode commit/barrier protocol and its
   parity harness. This is a prerequisite to broadening coverage because
   provenance, observation, retry, format, and checkpoint behavior cannot be
   retrofitted after the hot loop becomes authoritative.
3. Execute `.12`: move `MainControl` onto shared packed command episodes by
   semantic family. Canonicalize source/token frames, macro arguments,
   expansion, scalar scanners, conditions, and slow-helper resume; delete each
   predecessor as its family cuts over.
4. Execute `.13` after `.12`: make the mutable node builder canonical and
   freeze node/provenance/observation sidecars at the protocol's barriers.
5. Execute `.16` after `.13` to remove the comparison adapter, remaining
   coverage fallback variants, and unreachable scalar hot-path structures.
   This is required work, not optional cleanup.

The standalone region/cursor issue `.6` is subsumed by `.2` and this decision.
The shipout benchmark repair `.9` is independent tooling work and does not
block the engine architecture.

## Promotion gates

All ratios use the same release binary family, pinned source, format,
distribution, offline closure, profile, epoch, host affinity, and exact work
vector for both sides. A changed secondary work vector invalidates a CPU claim.
Use fresh processes and report median user CPU, `VmHWM`, allocation calls, and
requested bytes from the same complete timed region. Direct/nested slice rows
use at least three samples; pinned document rows use at least five
order-balanced samples. Report 6M and 12M endpoints so setup amortization is
visible. These gates compare within one benchmark contract, never across rows
in the evidence table.

### `.11`: sole state backing

- exact nested local/global assignments, same-level overwrite, global
  supersession, group tracing, rollback, state hash, snapshot/fork,
  incremental restore, loaded-format mutation, redump, and corruption tests;
- exact state, event, diagnostic, effect, artifact bytes, DVI, terminal, and log
  differential for direct 6M/12M and nested workloads;
- at least 10x user-CPU improvement at both direct endpoints and 5x nested;
- peak RSS no more than 50% of scalar stepping and allocation calls no more
  than 1% at both direct endpoints; and
- deletion of every private count/level/save/group array and synchronization
  path.

### `.15` and `.12`: barrier protocol and canonical command episodes

The `.12` closed-root cutover is now production-owned. Native `EngineSession`
startup registers one canonical `CommandState` root and retains a
capacity-only episode marker in `MainControl`; the live driver executes it
under the full command/state/mode aggregate transaction, publishes through the
ordinary artifact/DVI/effect/checkpoint seams, returns at output, and resumes
the same session to terminal. `umber2-uvfm` deleted the former admission
tokenizer, token vector, private cursor, macro/argument frames, and backup
slot. The standalone batch runner and benchmark engine selector are also
deleted. The historical pre-source-cutover medians were 32.8x at 6M fuel,
31.4x at 12M fuel, and 9.48x for nested forwarding, with episode RSS at
34.0%, 33.7%, and 42.2% of scalar stepping respectively. Canonical expansion
now exposes a real intermediate regression owned by `umber2-3gln`; no second
input or expansion path may be restored to recover those numbers. Focused
tests record zero character-profile and source-tokenization fallback across
all exact profiles and adversarial source/input levels. The detailed work
vectors and allocation counts are in
[`native_batch_kernel.md`](native_batch_kernel.md).

- the committed command-semantic fixtures, canonical event ordering with an
  observer, all scanner/condition/alignment/recovery cases, and exact
  diagnostic context pass through the episode path;
- `NeedResource`, cancellation, fuel, error, effect, checkpoint, and observer
  perturbation tests prove atomic rollback or commit and exact resume on the
  same engine;
- fresh and schema-12-loaded runs have identical state, output, format redump,
  and checkpoint schedules; cold and incremental runs compare state, effects,
  artifacts, dependencies, provenance queries, and named boundaries;
- the closed direct slice retains at least 10x at 6M/12M and 5x nested after
  canonical sidecars and barriers are enabled;
- the pinned production document at exact 6M/12M reaches at least 2x user-CPU
  improvement, no more than 70% of baseline peak RSS, and no more than 50% of
  baseline allocation calls and requested bytes; and
- every migrated semantic family records zero coverage fallback in its focused
  tests and deletes its old delivery/frame implementation before the next
  family is declared complete.

### `.13`: native node builders

- character, ligature, kern, glue, rule, box, paragraph, alignment, math,
  page-break, shipout, and rollback fixtures produce identical semantic node
  identities, artifacts, serialized bytes, DVI, and provenance queries;
- checkpoint capture before and after builder freeze, rejected revisions,
  output splicing, and loaded-format box roots retain exact ownership and
  bounded memory;
- the fixed node-construction workload reduces allocation calls and requested
  bytes by at least 80%, uses no more than 70% of baseline peak RSS, and is at
  least 2x faster; and
- the exact 6M/12M document gate from `.12` does not regress by more than 5%
  and no old persistent per-command builder remains for migrated node families.

The implementation deletes the superseded node vectors and passes the exact
semantic/output comparisons plus the allocation, RSS, and absolute-speed
gates. Its original guarded medians retained 27.5x/28.1x/8.93x speedups and
26.9%/27.0%/34.9% of scalar RSS. The linked `umber2-ujwo` repair then removed
the ordinary fresh-shipout reparse: `tex-out::DviPagePlanCoEmitter` consumes the
same scalar events as the canonical artifact encoder while the immutable node
root is borrowed. Artifact bytes remain the sole serialized authority and the
operation-local plan retains no `PageNode` or engine handle. DVI leaders, whose
semantics require subtree replay, switch only that operation to the existing
bounded canonical-byte adapter rather than retaining a compatibility tree.

A same-host rebuild of the actual `.12` commit (`faf269be4`) and the repaired
tree used one release binary per revision, fresh guarded processes, CPU 4
affinity, identical work vectors, and five order-balanced samples per row. The
`.12`/repaired medians were 258.668/263.858 ms at 6M (+2.01%),
498.207/521.703 ms at 12M (+4.72%), and 197.083/198.514 ms for nested forwarding
(+0.73%). Exact state, artifact, DVI, effect, terminal, log, and fuel comparison
passed at all three points. The repaired direct rows requested
156,407,429/310,320,757 bytes versus `.12`'s
182,893,123/363,300,819 (-14.48%/-14.58%), so the five-percent stage gate is
closed without restoring a parallel node authority.

### Final deletion and compatibility

`cargo test -q --tests` must pass after every stage. The committed canonical
command fixtures are the first-divergence oracle; transcript/log/DVI TRIP and
e-TRIP tiers, loaded Plain/e-TeX/LaTeX/pdfLaTeX formats, incremental/cold
equality, provenance queries, and PDF/DVI artifact gates retain their existing
tier and authority. Expensive live regeneration continues only through
`scripts/regen-fixtures.sh`; architecture promotion never treats an absent or
ignored oracle as a pass.

The final deletion stage reruns the `.12` exact-work performance gates and the
complete correctness hierarchy. It is complete only when the static deletion
criteria above hold, not merely when the fast path is the default.

## Implemented convergence

Issue `umber2-c1p8` removed the remaining closed-root executor. Production no
longer contains `NativeBatchProgram`, its private control dispatcher or
scanner grammar, `CountGroupEpisode`, its character/kern sink, synthesized
shipout, packed-root continuation, or coverage-fallback telemetry. All roots
run the complete static profile vocabulary through the bounded
`MainControl::advance_episode` loop and its sole aggregate rollback owner.

Required semantic barriers remain. Group-lineage and nonrollbackable changes
are internal early-commit reasons, not selection of another executor. The
historical `.8`--`.13` rows therefore remain prototype and migration evidence;
the maintained performance workload is `canonical_episode`, which measures
the converged production route.
