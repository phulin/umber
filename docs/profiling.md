# Profiling Umber with Gentle

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations, full debug
information, and the compile-time `profiling-stats` instrumentation, records
50 measured executions with Samply, and writes the profile to
`target/profiles/gentle.json.gz`. Samply also writes its
presymbolication sidecar next to the profile when supported. Override the run
counts and output path with:

```bash
GENTLE_PROFILE_ITERATIONS=100 \
GENTLE_PROFILE_WARMUPS=2 \
GENTLE_PROFILE_OUTPUT=/tmp/gentle.json.gz \
scripts/profile-gentle.sh
```

Extra arguments are forwarded to the runner. Run the optimized workload
without Samply when checking timing or setup:

```bash
cargo run --profile profiling -p umber --bin gentle-profile \
  --features profiling-stats -- \
  --iterations 10 --warmups 1
```

Pass `--checkpoints` to exercise the enabled named-boundary capture path. The
runner consumes every published checkpoint and folds its semantic hash into a
bounded observation instead of retaining snapshots across iterations:

```bash
GENTLE_PROFILE_ITERATIONS=200 scripts/profile-gentle.sh --checkpoints
```

Pass `--incremental-edit` to measure a fixed semantic prose edit 20% through
`gentle.tex`. Every sample keeps one session alive for four accepted revisions:
the pinned large edit, a follow-up insertion in that paragraph, removal of the
follow-up, and an equal-width word substitution. Adjacent disabled/enabled
samples alternate AB/BA order, so the iteration count must be even. The runner
reports paired latency differences for each revision and separate aggregate
totals for three paths. The large insertion and inverse removal are the slow
paragraph path: pagination changes and neither policy may adopt a suffix. The
follow-up insertion is the interaction path: both policies must publish the
same named-boundary schedule and retain/re-ship/adopt the same page counts
after earlier paragraph hits. The equal-width substitution is the fast path:
both policies must preserve the same schedule, reconverge at shipout, re-ship
the pinned three changed pages, and adopt every page in the unchanged suffix.
Both modes must also produce the exact DVI bytes of a fresh cold compile for
every revision. The summary reports steady-state slow, interaction, and fast
paired totals, a priming-inclusive slow total, boundary-schedule equivalence,
page reuse, trace hits, and incremental-to-cold latency ratios:

```bash
cargo run --profile profiling -p umber --bin gentle-profile -- \
  --repo-root /path/to/umber2 --incremental-edit --iterations 6 --warmups 1
```

Use the build without `profiling-stats` for release latency. The first summary
line prints `profiling_stats=false` so an attributed diagnostic run cannot be
mistaken for the release comparison. Rebuild with `--features profiling-stats`
only when the named recording phases and exact-identity counters are needed to
explain a losing path.

Select recording layers with `--memo-layers`. For this profiler's explicitly
memo-enabled candidate, the default is `paragraph`, whose results and trace
metadata belong to the accepted generation. The engine memo runtime remains
disabled by default; this profiler policy is an experimental comparison, not
the batch or editor-session product default. Explicit experiments may select
comma-separated `pretolerance,paragraph,page,shipout`, `all`, or `none`.
Detached layers are off by default until they demonstrate steady-state value
and budget fit.

For a direct marginal comparison between two recording policies, pass
`--baseline-memo-layers` as well. Both policies then run inside the same
alternating AB/BA loop, and the paired delta is reported as candidate minus
baseline. For example, the isolated pretolerance comparison uses:

```bash
cargo run --profile profiling -p umber --bin gentle-profile -- \
  --repo-root /path/to/umber2 --incremental-edit --iterations 6 --warmups 2 \
  --baseline-memo-layers paragraph \
  --memo-layers pretolerance,paragraph
```

This mode retains the per-revision cold-DVI comparison and suffix-adoption
assertions. It reduces process-level drift but does not make samples taken
during thermal pressure or unrelated host contention admissible.

For every accepted edit, each layer reports lookups, hits, inserts, evictions,
retained bytes, and misses split into not attempted, ineligible barrier, key
miss, first validation failure, evicted before reuse, and import failure.
Paragraph barrier reasons and the first failing dependency family are printed
separately. Record, lookup, validation/key construction, and import time are
reported independently. Generation-anchored paragraph metadata bytes are
reported separately from detached-cache bytes.

The fixed edit inserts 1,792 words into one paragraph beginning 19.66% through
the source. It deliberately changes both line and page breaking: with corrected
line-break widths the pinned document grows from 97 to 100 pages, and the
downstream 86-page region is retyped rather than adopted. A five-sample
optimized run on 2026-07-15, before that width correction, measured 3.986
seconds mean (2.940 seconds median) with memoization disabled, 7.304 seconds
mean (7.222 seconds median) with memoization enabled, and 2.875 seconds mean
(2.740 seconds median) for a cold compile of the edited document. Memoization
was therefore 83% slower by the means and 146% slower by the medians. It made
7,140 lookups but only 385 hits, reached 67,008,455 retained bytes, and evicted
6,475 entries.
Those absolute timings predate the current fixture output and are historical
only. Both current incremental modes produce the cold compile's exact 100-page,
279,176-byte DVI. This is an intentional page-divergence stress case, not the
expected case for suffix reuse.

After stable pre-delivery paragraph anchors landed, an independent five-sample
rerun on 2026-07-16 (also before the width correction) raised the downstream
eligible paragraph result to 121 of 129 candidates (93.8%). Memo-enabled
execution still lost to memo-disabled execution: 9.446 versus 6.409 seconds by
the means and 8.562 versus 2.967 seconds by the medians. The general detached cache retained
66,899,304 bytes and evicted 6,721 entries; page episodes made 5,378 lookups for
30 hits on the deliberately pagination-shifting edit. These historical
observations did not support a per-layer verdict: they used weak unpaired
samples, measured only the first accepted edit, and bundled recording with
reuse. They remain provenance for the instrumentation change, not the current
release conclusion. The runner now supplies the missing taxonomy, phase
timings, steady-state edits, and paired ordering described above. The
standalone expansion-episode cache was removed after repeated zero-traffic
measurements; pretolerance remains an opt-in experiment after its isolated
comparison failed to establish a removal win.

The first post-main run with the complete methodology used six optimized AB/BA
pairs. Memo-disabled versus paragraph-memo means were 1,700.926 versus
1,776.852 ms for the large edit, 1,671.180 versus 1,567.323 ms for the follow-up,
and 1,689.310 versus 2,043.332 ms for its removal. The paired enabled-minus-
disabled means were therefore +75.926, -103.857, and +354.022 ms. Every
revision remained byte-identical to its cold 100-page DVI: 279,176 bytes for
the large insertion and inverse removal, and 279,248 bytes for the follow-up.
The first two edits
made 121 and 122 finished-line hits and spent only 9.959 and 10.008 ms in
validation plus import, but the removal attempted no paragraph lookup, reported
1,781 not-attempted regions, and retyped the complete document. Paragraph
generation metadata grew from 19.2 MiB to 19.8 MiB and then 23.6 MiB; detached
retention and evictions remained zero under the default paragraph-only policy.
This is a negative release result even though the validation/import cost clause
passes on revisions that engage lookup.

`commands_reexecuted` is the number of tokens that reached scalar
main-control dispatch; it is not a count of paragraph preflight probes and it
is not interchangeable with `tokens_reexecuted`. The latter also includes
ordinary character tokens consumed by the macro-body and physical-source text
span paths. Incremental work output therefore reports
`macro_text_span_tokens` and `source_text_span_tokens` beside the command
count.

A focused audit of the Gentle large-edit anomaly found real dispatch work, not
double accounting. With memoization disabled, main control can consume much of
a literal paragraph directly from physical-source text spans. A paragraph
memo lookup must first scan a candidate; after a key or validation miss it
pushes those traced tokens back for normal execution. That replay preserves
semantics and provenance but is no longer a physical-source span, so the same
characters take the scalar dispatch path. A two-run AB/BA diagnostic at the
post-`vfqs.19` baseline reproduced 129,370 accounted tokens, 41,334 scalar
dispatches, 3,641 macro-span tokens, and 84,395 source-span tokens disabled.
The enabled side reported 129,906 accounted tokens, 71,776 scalar dispatches,
3,358 macro-span tokens, 52,023 source-span tokens, and 2,749 skipped paragraph
commands. These buckets reconcile exactly on both sides: the disabled token
total is scalar plus both span paths, while the enabled total also includes
the skipped hit traces. The approximately 30,000-command delta is consequently
attributable to paragraph-preflight miss replay and lost span batching.
Paragraph hits remain excluded from the dispatch count; their avoided commands
continue to be reported separately as `commands_skipped`.

The fixture now appends a fourth, height-preserving edit after that inverse
removal. It changes `words` to `sword`; the two words contain the same cmr10
glyphs, retain the same `wo` kern, and introduce no other kern or ligature pair,
so the changed page has different content but identical broken height. A
two-pair balanced acceptance run on 2026-07-16 retyped three pages, performed
one exact-state check without memo recording (two with paragraph recording),
and adopted the remaining 83 pages in both modes. Mean latency was 66.550 ms
disabled, 103.070 ms enabled, and 1,762.488 ms cold. The revision was
byte-identical to its cold 100-page, 279,176-byte DVI. This small run verifies
the fast-path workload and parity; it is not a standalone latency verdict.

The lazy exact-identity stage was then checked with the same four-edit fixture
in a two-run balanced smoke measurement. Cold history computed zero exact
identities. The two full re-executions requested 73 comparisons each instead
of publishing identities at roughly 1,100 named boundaries, and the
height-preserving fourth edit requested one comparison, retyped three pages,
and again adopted 83. Memo-disabled means for the four edits were 254.056,
759.960, 349.745, and 89.085 ms on that run, with every revision byte-identical
to its cold DVI. Immutable store content is cached separately from mutable
checkpoint state and compact page identity no longer serializes the full font
bundle at every comparison. Treat these two samples as implementation and
work-count evidence, not a conditioned latency verdict.

The composed-root stage was checked on 2026-07-16 with the same two-pair
balanced four-edit run and the profiler's dedicated exact-identity timer. The
inverse third edit performed 73 memo-disabled identities in 0.845 ms total and
70 memo-enabled identities in 0.909 ms after their component roots were cached.
The height-preserving fourth edit performed one identity in 0.672 ms disabled
and two in 0.852 ms total enabled, retyped three pages, adopted the remaining
83-page suffix, and produced the cold 100-page, 279,176-byte DVI in both modes.
Its mean total latency was 65.322 ms disabled, 101.352 ms enabled, and 439.497
ms cold. The follow-up full re-execution also exposes the first-projection cost:
146/139 identities totaled 84.456/88.113 ms while populating previously unseen
accepted component roots. Mutable-store and page DTO serialization is absent;
the counter measures only cached-root projection and composition. Treat the
two samples as asymptotic and work-accounting evidence, not a stable machine
benchmark.

The hierarchical-trace acceptance rerun on 2026-07-16 added explicit
prefix/re-ship/suffix accounting and leaf/subtree telemetry. In both modes the
fourth edit retained the first 14 pages, re-shipped exactly three pages through
the matching `ShipoutComplete`, and probabilistically adopted all 83 remaining
pages as one subtree through the authoritative session-local 64-bit aHash
comparison. The observed output remained byte-identical to the cold 100-page,
279,176-byte DVI. Over two AB/BA pairs, memo-disabled incremental execution
averaged 146.774 ms, paragraph-recording execution averaged 225.502 ms, and
cold execution averaged 1,978.659 ms: 0.074x and 0.114x cold latency. Treat the
small sample as fast-path work and parity evidence, not a conditioned latency
verdict.

A separate two-pair diagnostic enabled `pretolerance,paragraph`. Pretolerance
reported 834/835, 833/834, and 1,054/1,054 hits over the three edits, retained
about 200 KiB, and evicted nothing. Treat this only as evidence that the layer
has traffic; the sample is too small and does not isolate its marginal latency.
Conversely, two disabled and two enabled ten-run whole-Gentle blocks again
reported zero expansion-episode lookups, hits, entries, retained bytes, and
evictions while preserving the pinned 97-page/263,424-byte DVI. The expansion
layer had no measured signal and was removed with its profiler option and
counters.

The follow-up bounded ABBA comparison ran two-iteration incremental blocks for
paragraph-only (A) and paragraph-plus-pretolerance (B), with every revision
checked against cold DVI and the fourth edit still adopting the 83-page suffix.
The first A/B pair favored B on every edit, but the reverse B/A pair lost by
large margins during visible host contention. Pretolerance consistently made
936/937 and 937/937 hits on the large and inverse edits, retained about 200
KiB, and paid measurable validation cost. Because the paired direction
reversed, the result is not evidence that removal is neutral or positive;
pretolerance remains opt-in pending a conditioned rerun.

The final paragraph-only release pass on 2026-07-16 used four AB/BA-paired
samples after one in-process warm-up. The paired enabled-minus-disabled means
for the large insertion, follow-up insertion, inverse removal, and
height-preserving substitution were respectively +768.447, +207.602,
+923.535, and +266.583 ms. The host was noisy (some individual pairs reversed
direction), but the bounded order-balanced result does not support enablement:
paragraph recording lost on every paired mean. The corresponding disabled /
enabled / cold means were 1329.144/2097.590/689.275,
2738.331/2945.933/734.451, 1086.527/2010.062/814.330, and
245.181/511.763/725.040 ms.

All eight incremental outputs were byte-identical to their fresh cold DVI:
the four revisions emitted 100 pages and 279,176, 279,248, 279,176, and
279,176 bytes. Restart/suffix accounting was respectively 14/86/0,
14/13/73, 14/86/0, and 14/3/83 retained-prefix/re-shipped/adopted pages. The
fourth edit reconverged at `ShipoutComplete`, walked one subtree, and adopted
all 83 suffix leaves. Paragraph validation plus import remained cheaper than
front-end execution in the measured enabled sample: 14.795/584.845,
3.744/2166.775, 43.836/1899.058, and 0/362.014 ms. Paragraph lookup
hit/key-miss/validation-miss counts were 19/1384/1, 3/79/0, 19/1385/0, and
0/5/0; import failures were zero. The matching validation-eligible candidates
therefore cleared 90% on every edit that reached validation, while 648, 51,
648, and 2 executed regions recorded barriers. The large and inverse edits
each attributed 50 display, 47 output-routine, 219 unsupported-write, 255
unsupported-input-transition, and 540 group-transition barrier events.

The enabled exact-identity timer reported 2/6.213 ms, 127/1163.022 ms, 0/0,
and 4/43.142 ms (calls/total) across the four edits. The high many-comparison
sample is first-projection and host-contention sensitive, so it is work/cost
telemetry rather than a standalone hash verdict. Paragraph generation
metadata retained 19,112,980 bytes; detached-cache retention and evictions
remained zero. The committed edit matrix, external Story/Gentle/TRIP/e-TRIP
parity, explicit 1,000-edit tier, snapshot budgets, and complete workspace
test/format/clippy gate all passed. The release blocker is solely that
memo-enabled recompilation did not beat memo-disabled recompilation; no broad
tuning or optional pretolerance policy is included in this verdict.

The exact-projection follow-up retained fixed-size component roots with
snapshots and separated the immutable name/token/macro/glue/font collection
caches by allocator lineage. In the single bounded two-pair AB/BA verification,
the memo-enabled follow-up edit's 127 exact identities fell from the local
pre-change 388.011 ms to 8.834 ms. Component projection builders accounted for
381 calls, 127 visits, and 0.166 ms; immutable collections reported 560 root
hits, 75 dirty or previously unseen root misses, and 2,194 newly composed
leaves. Exact identity therefore no longer dominates enabled latency. All four
outputs remained byte-identical to their cold 100-page DVI, and the fourth edit
retained 14 pages, re-shipped three, and adopted the 83-page suffix. Host load
made end-to-end paired deltas directionally unstable; the remaining paragraph
memo reexecution overhead is tracked independently by `umber2-vfqs.15.4.2`.

The final independent parent acceptance rerun after all seven attribution
children used one bounded four-pair, same-process AB/BA comparison with one
warm-up and the default paragraph-only policy. Enabled-minus-disabled paired
means for the large insertion, follow-up insertion, inverse removal, and
height-preserving substitution were +74.281, +31.387, +65.301, and +10.855 ms.
Memo-enabled recompilation therefore still lost on every edit. The additive
stage split assigned +58.597, +28.515, +51.102, and +9.298 ms to the executor;
the named paragraph recording phases accounted for about 20.896, 3.265,
22.324, and 1.597 ms. The remaining cost is executor-side distributed
necessary recording and allocation rather than validation/import, exact
identity, or a newly independent dominant micro-path.

Paragraph hit/key-miss/validation-miss counts were 19/1,384/1, 3/79/0,
19/1,385/0, and 0/5/0, with no import failures. Candidates that reached
validation therefore hit at 95%, 100%, and 100%; the fourth edit had no
validation-eligible candidate. Validation plus import cost 1.059, 0.159,
1.080, and 0 ms, far below enabled executor time. Recorded barrier-region
counts were 648, 51, 648, and 2. Exact-identity calls/time were 2/5.081 ms,
127/3.764 ms, 0/0, and 4/5.284 ms. Generation metadata retained 19,112,980
bytes; detached retention and evictions stayed zero.

All revisions matched cold output exactly at 100 pages and
279,176/279,248/279,176/279,176 DVI bytes. Restart accounting was
14/86/0, 14/13/73, 14/86/0, and 14/3/83 retained/re-shipped/adopted pages;
the second and fourth edits adopted one suffix subtree with 73 and 83 leaf
hits. The committed edit matrix, all four external corpus cases, explicit
1,000-edit tier, snapshot budgets, workspace tests, rustfmt, dprint, and clippy
passed. The release gate remains failed solely because recording paragraphs
does not beat the disabled control.

The parent capability evaluation therefore closes with a split result:
macro-bearing reuse, validation/import savings, parity, and the eligible-hit
target are delivered, but the end-to-end speed criterion failed. Paragraph
memoization remains default-disabled, and the encompassing release review must
consume this as a negative default-enablement decision rather than a passed
performance acceptance.

The incremental comparison now times the initial accepted-generation compile
as well as each of its four accepted edits. AB/BA ordering applies to the
complete same-process sequence, and the report prints both per-edit paired
deltas and a baseline-inclusive paired total. Paragraph opportunity telemetry
is emitted for priming and every edit as `regions/bytes/nanos` for census-only,
fully armed, carried-forward, seeded, published, and declined work. This keeps
a recording policy from hiding seed cost or trace attrition in an unreported
baseline.

The paragraph-reexecution follow-up first reproduced the delivery-path shift
in a direct two-run AB/BA same-process comparison. The large edit moved from
41,334 scalar commands and 84,395 physical-source span tokens disabled to
71,776 commands and 52,023 span tokens enabled; the inverse edit reproduced
the same counts. Paragraph lookup, record, validation, and import telemetry
totaled about 17 ms and 23 ms on those enabled edits, far below the observed
reexecution loss, while exact identity remained bounded by the retained-root
cache. This attributes the dominant independent cost to preflight miss replay
destroying source-span batching rather than to authoritative exact equality.

Preflight pushback now uses a dedicated transient replay kind. Horizontal main
control batches its original traced `Letter`/`Other` words without retokenizing
or changing expansion, provenance, recording, rollback, or input semantics. A
four-run AB/BA comparison reduced enabled scalar commands to 40,800 on
the large and inverse edits while restoring 82,999 source-span tokens. The
memo-disabled buckets remained 41,334/84,395; the enabled token total still
includes 2,749 commands skipped by paragraph hits. All revisions remained
byte-identical to cold DVI, and the fourth edit retained 14 pages, re-shipped
three, and adopted the 83-page suffix. Enabled still lost by paired means of
282.651 and 264.070 ms on the large and inverse edits despite only about
13--15 ms of reported paragraph phases. The remaining uninstrumented
recording/execution cost is therefore a separate bottleneck, not grounds to
widen this optimization or alter exact-state equality.

The next pass added profiling-only named recording phases. A four-pair AB/BA
baseline attributed about 205 ms of the large and inverse edits to break
dependency capture, versus roughly 2.4 ms for trace capture, 7--8 ms for
front-end provenance, 9 ms for front-end dependencies, and 2--4 ms for retained
graph work. A further two-pair split measured only 0.3 ms discovering break
keys and 0.5 ms registering stamps; semantic-value projection consumed
204--209 ms. The implementation therefore caches at most 4,096 detached break
observations inside one `ExecutionContext`, reusing an observation only under
exact changed-at equality and recomputing through `Universe` after any stamp
change.

The four-pair post-change run reduced break semantic projection to
18.6--18.8 ms on the 894-paragraph large and inverse edits. Scalar commands and
source-span delivery remained 40,800/82,999, all revisions stayed
byte-identical to cold DVI, and the fourth edit again retained 14 pages,
re-shipped three, and adopted 83. The end-to-end sample remained visibly noisy
and retained a residual larger than all named recording phases; follow-up
attribution is `umber2-vfqs.15.4.4`, not part of this measured fix.

The residual-attribution follow-up timed the complete incremental acceptance
path additively. Its final four-pair AB/BA run reported enabled-minus-disabled
means of +153.730, +86.131, +134.716, and +44.486 ms across the four edits.
On the large and inverse edits, respectively, executor work accounted for
+136.465 and +118.291 ms, splice/history construction for +4.664 and
+5.864 ms, paragraph-generation publication/drop for +1.219 and +1.247 ms,
and the then-combined substrate acceptance/drop bucket for +10.290 and
+8.781 ms. Revision setup, diagnostics/effect snapshots, DVI materialization,
and the additive unaccounted remainder were each below 1 ms of paired delta.
The runner now splits accepted-substrate publication/drop from pruning and
accepted-output construction so later captures retain that distinction.

Within the executor, profiling-only named recording phases totaled about
33--35 ms on those edits. Sparse input-stack attribution counted 88,432 stable
source-recording probes but estimated only 1.6 ms of work; paragraph validation
and import remained separately reported by the memo layer. The 130,933 named
timer samples had a calibrated measurement floor of about 4.45 ms. Thus the
post-projection residual is real executor-side recording/allocation work, not
diagnostics, generation publication, DVI construction, or an unmeasured wall
clock gap, but no single additional owner was independently dominant enough
for a principled semantic-preserving optimization. A sampled mixed-mode run
charged 22.8% of self samples to the system allocator, distributed across many
callers rather than one removable allocation. No optimization was retained.
All outputs remained byte-identical to cold, inverse restart stayed exact, and
the final edit again retained 14 pages, re-shipped three, and adopted 83.

Executor-internal sampling then isolated the missing owner in macro-bearing
paragraph publication. After deriving root spans for the expanded trace, the
recorder merged them into the ordered consumed-source sequence by linearly
scanning every span already present. The two hottest offsets inside
`publish_prepared_hlist` were that inlined equality loop, with 386 self samples
in the bounded diagnostic capture. The existing front-end-provenance timer had
ended immediately before the merge, so its reported 4.6--5.8 ms excluded the
quadratic work. Publication now keeps the same first-occurrence order required
by monotonic input-transition validation but maintains a temporary membership
set while appending unseen spans. The provenance phase timer covers both root
projection and this ordered merge.

One final four-pair AB/BA run reduced the large/inverse executor deltas from
the local pre-change two-pair diagnostic's +119.826/+111.775 ms to
+84.794/+77.081 ms. End-to-end enabled-minus-disabled deltas were +102.076,
+38.592, +92.946, and +9.455 ms across the four edits. Front-end provenance,
now including the complete merge, measured 7.388 and 8.601 ms on the
130,933-token large/inverse paths. Every result remained byte-identical to its
cold DVI; the final edit retained 14 pages, re-shipped three, and adopted all
83 suffix pages. This is a material reduction from one measured optimization,
not a release win; the remaining executor residual is tracked by
`umber2-vfqs.15.4.6`.

The next residual pass compared two bounded non-instrumented samples with both
incremental policies set to paragraph recording against two with both policies
disabled. The newly dominant paragraph owner was dependency validation under
`try_reuse_literal_paragraph`: its 144 inclusive samples repeatedly projected
the complete unchanged hyphenation-pattern fingerprint. The existing bounded
execution-local paragraph observation cache already guarded break capture by
exact changed-at equality. Validation now consults that same cache and inserts
an authoritative projection on a miss; any stamp change still recomputes
through `Universe`, and the cache remains outside generations and replay.

The single final four-pair AB/BA comparison reduced paragraph validation on the
large/inverse edits from the local pre-change 10.745/10.670 ms to 4.834/4.829
ms and reduced executor deltas from +84.794/+77.081 ms to +74.663/+68.110 ms.
End-to-end deltas were +91.464, +43.781, +82.353, and +8.508 ms. All revisions
remained byte-identical to cold DVI, dependency and barrier counts were
unchanged, and the fourth edit retained 14 pages, re-shipped three, and adopted
all 83 suffix pages. The still-material executor residual is a separate
follow-up rather than grounds to add another optimization to this pass.

A bounded non-instrumented same-policy differential attributed the next
executor residual to the first break-dependency projection after restart. The
paragraph-enabled profile contained 705 samples below paragraph publication;
263 (37%) walked `HyphenationTable::dependency_fingerprint`, while the disabled
profile had no paragraph-publication traffic. The existing execution-local
cache prevented repeated projection within one advance, but every checkpoint
fork still rescanned the unchanged pattern trie. Hyphenation copy-on-write
roots now retain this derived per-language projection. Mutations invalidate the
new root's cache, and serialization, semantic equality, changed-at validation,
and detached observations do not depend on it.

The single final four-pair AB/BA run measured 0.883/0.906 ms of complete break-
dependency capture and 0.346/0.353 ms of value projection on the large/inverse
edits, down from the prior 18.6--18.8 ms projection totals. Executor deltas fell
from +74.663/+68.110 ms to +60.736/+53.322 ms; end-to-end deltas were +78.227,
+43.475, +67.372, and +7.792 ms across the four edits. All outputs remained
byte-identical to cold DVI, and the fourth edit retained 14 pages, re-shipped
three, and adopted all 83 suffix pages.

The compositional follow-up separates pagination-changing slow edits from
suffix-preserving interaction and height-preserving fast edits. Its final
release build used eight AB/BA pairs after two warm-ups. Enabled-minus-disabled
means were +33.048 ms for the large insertion, +26.295 ms for the
suffix-preserving follow-up, +34.547 ms for the inverse removal, and +0.960 ms
for the height-preserving substitution. The two slow edits therefore lost
67.594 ms in steady state and 110.206 ms including the 42.612 ms initial
recording cost. All four baseline and candidate boundary schedules were
identical. The fast edit retained 14 pages, re-shipped three, adopted 83 as one
subtree, and measured 0.280x/0.284x cold latency without/with paragraph
recording.

On the slow edits, 137 fully armed opportunities produced 27 hits (19.7%) and
skipped 3,851 commands, about 4.5% of the combined dispatched-plus-skipped
work. Validation and import totaled only 1.432 ms. Additive paired ownership
assigned 40.955 ms to executor work, 8.904 ms to splice construction, and
15.628 ms to acceptance; the remaining named stages were individually below
1 ms combined except restart fork. Priming published 512 seeded regions and
13,543,881 bytes of paragraph metadata. A bounded hot-path change replaced
per-read B-tree insertion with append-only paragraph dependency capture and one
sort/dedup at paragraph publication. In matched four-pair instrumented runs,
the combined slow executor delta fell from 49.507 to 38.593 ms and the slow
end-to-end delta from 77.347 to 66.960 ms. A separate acceptance/pruning
hypothesis did not affect the measured path and was discarded. The retained
optimization is material, but sparse useful coverage still cannot repay
recording and generation costs, so paragraph memoization remains
default-disabled.

The runner requires the same external inputs as Gentle conformance. Populate
them with `scripts/setup-conformance-tests.sh` if necessary.

At startup, the runner reads `gentle.tex`, `plain.tex`, `hyphen.tex`, and the
available Computer Modern TFM files into a memory-backed `World`. Seeded input
bytes are structurally shared by every fresh run. One warm-up and all measured
iterations execute in the same process without temporary directories, corpus
copies, or host file reads. Each iteration still includes normal engine input
opening and hashing through `World`, engine initialization, expansion,
execution, shipout, and final DVI generation. After sampling, the runner checks
the final DVI against the warm-up result.

The optimized runner must produce 97 pages and a 263,424-byte DVI for the
current pinned Gentle corpus. That byte count is also the size of
`tests/corpus/e2e/gentle.expected.dvi`; the strict conformance test remains the
authoritative byte-for-byte check. Profiles captured before the macro-text
release-path repair consumed lexer spans from inside a `debug_assert!`, so the
side-effecting character append was compiled out and those runs produced an
incorrect 257,304-byte DVI. Do not compare their absolute timings or hotspot
weights with corrected captures. The first corrected scalar 200-run capture is
`/tmp/gentle-q50-correct-scalar-baseline.json.gz` (92.351 ms/run on the capture
machine).

The ordinary end-to-end Gentle conformance test remains the correctness and
host-workflow measurement. This profiling runner deliberately removes its
repeated staging, oracle reads, artifact writes, and temporary-directory
cleanup so those operations do not obscure engine hotspots.

## Expansion meaning-site cache evidence

The expansion meaning-site cache is guarded by the owning `Stores` identity
and a monotonic meaning-write generation. A 200-iteration corrected Gentle run
after the expanded-replacement span fix produced 97 pages and 263,424 DVI
bytes, with 20,240 cache hits and 57,307 misses at guarded macro-body sites
(26.1% hits). Profiling-only invalidation counters over the warm-up plus 200
measured runs recorded 448,431 local meaning writes, 21,507 global meaning
writes, and 2,217,432 conservative group-exit invalidations.

Returning one `meaning_changed` bit from Env group restoration to the owning
`Stores` boundary lets empty and non-meaning groups retain valid entries while
both group-exit paths still invalidate whenever their journal restores or
compacts a meaning cell. The corrected 200-iteration rerun increased reuse to
40,953 hits against 36,594 misses (52.8% hits) and reduced group-exit
invalidations to 91,857, while retaining the same 97 pages and 263,424 bytes.
Local/global writes, rollback, owner isolation, and both group-exit paths have
focused invalidation coverage; debug cache hits also compare against the live
aggregate meaning.

After moving the cache and current replay-site state from `tex-lex::InputStack`
to the persistent `tex_expand::ExpansionContext`, a corrected 200-iteration
run retained 40,925 hits and 36,650 misses, completed at 118.061 ms/run on the
capture machine, and again produced 97 pages and 263,424 bytes. The lexer now
delivers only semantic-free macro replay-site metadata; a compile-fail boundary
test prevents `InputStack` from regaining meaning resolution.

The conditioned `BOOB` plus five `BOOBOBBO` paired comparison was noisy but
flat: refined versus conservative raw means were 134.191 and 134.350 ms/run,
medians were 118.299 and 117.795 ms/run, and means after excluding the two
greater-than-200-ms host-contention outliers on each side were 119.857 and
119.383 ms/run. The selective policy is retained because it removes needless
invalidation through a small exact journal-owned signal, materially improves
cache reuse, and shows no meaningful throughput regression; the cache itself
remains justified by the corrected 52.8% guarded-site hit rate.

## Physical-source text-run evidence

Horizontal main control now has a guarded physical-source path alongside the
existing immutable macro-replay span path. It accepts only directly backed
`Letter` and `Other` scalars under their current catcodes; all lexer-semantic,
provenance, tracing, alignment, and source-frame boundaries remain scalar.
A corrected 200-iteration Gentle run produced the pinned 97 pages and 263,424
DVI bytes, with 20,436 accepted runs containing 89,522 tokens (4.381 tokens per
run) from 48,407 source-path probes, a 42.2% accepted-run rate.

The same conditioned `BOOB` plus five `BOOBOBBO` comparison was throughput
flat across 22 ten-iteration samples per binary. The pre-change baseline mean
and median were 112.247 and 111.843 ms/run; the source-run candidate measured
112.263 and 112.003 ms/run, a -0.014% mean change. The path is retained as an
exact, localized reduction in per-token expansion delivery rather than a
claimed end-to-end speedup. Focused tests cover dynamic catcode and UTF-8
cursors, `^^` and alignment deoptimization, tracing, summary restoration, and
the precise provenance seam; strict Gentle remains byte-identical.

## Analyze a capture

Use the repository analyzer for a repeatable text report instead of manually
expanding stacks in the Samply UI:

```bash
scripts/analyze-profile.sh
scripts/analyze-profile.sh --top 40 target/profiles/gentle.json.gz
```

The report ranks self time and recursion-deduplicated inclusive time. It also
separates runtime self samples by library and attributes them to the nearest
Umber frame, which makes allocator and memory-operation costs visible without
losing their application caller. Percentages use Samply sample weights rather
than assuming every sample has weight one.

For a focused question, restrict the report to stacks beneath a function. The
subtree report adds the function's immediate callees, immediate callers, and
nearest non-runtime application callers. This assigns allocator or memory
runtime to the engine operation above it while still showing both the share
within the subtree and the share of the complete capture:

```bash
scripts/analyze-profile.sh \
  --subtree drain_pending_output \
  target/profiles/gentle.json.gz
```

Samply normally writes `gentle.json.syms.json` beside `gentle.json.gz`; the
analyzer discovers that sidecar automatically and uses its exact address map,
including inline frames. Pass `--symbols PATH` for a sidecar elsewhere. If no
sidecar exists, already-symbolized profile names remain useful and unresolved
addresses are reported explicitly rather than guessed. Use `--thread TEXT` or
`--app TEXT` when automatic thread or application-library selection is
ambiguous, and `--json` for machine-readable output.

Compiler inlining can make broad entry-point frames such as `main` or `run`
dominate a whole-profile inclusive ranking. Self time is unaffected; use a
named subtree when comparing the internal costs of a subsystem.

## Checkpoint optimization evidence

Measure checkpoint changes with both a 200-iteration Samply capture and a
thermally conditioned, interleaved wall-clock comparison. Use one warm-up and
ten measured runs per invocation; condition with `BOOB`, then measure
`BOOBOBBO` five times. Compare checkpoint-enabled binaries first and run the
disabled control only for a candidate that improves enabled timing. This
pairing is necessary because sub-percent Samply movements on thermally
constrained machines have repeatedly failed to predict throughput.

After the page forest and mutable tail gained structural lazy projections, a
200-run Gentle capture placed the complete `EngineSession::publish` subtree at
about 3.4% of whole-run samples. Its remaining work was fragmented across the
journal (about 0.6%), current page (about 0.6%), input hashing (about 0.4%), and
input summary construction and validation (about 0.4%). Experiments that
removed a sampled clone/drop, recursively composed page-tail roots, fused input
validation with projection, or changed canonical byte decoding were flat or
worse under the paired evidence. Treat those residual entries as attribution,
not independent optimization budgets: moving work between their inline frames
does not by itself demonstrate a faster checkpoint path.

The corrected 263,424-byte workload exposed a larger shared cost below those
components: `StateHasher` previously ran a complete two-multiply SplitMix64
avalanche for every canonical scalar and then finalized with another avalanche.
Checkpoint hash schema version 8 replaces the per-field avalanche with a
target-independent ordered one-multiply recurrence while retaining SplitMix64
as the finalizer. A corrected 200-run checkpoint profile reduced publication
from 3.46% to 2.71% and mean time from 96.846 to 92.641 ms/run. Conditioned
`BOOB` plus `BOOBOBBO` five times confirmed 93.607 to 89.704 ms/run with
checkpoints (4.17%) and 89.847 to 86.762 ms/run without checkpoints (3.43%);
wall-clock deltas were 4.35% and 3.74%, respectively.

## Node-sidecar allocation evidence

After content identity v2 removed the earlier hash hotspot, caller
reconstruction placed `RawVec::finish_grow` at 8.40% of a 200-run default
Gentle profile. The largest coherent owner was compact node storage, including
0.56% directly under the nine-column box-sidecar reserve. Boxes are consumed
as complete values in the current execution, packing, copy, and shipout paths,
so row-packing that one sidecar removed the independent box reserve frame and
reduced `finish_grow` to 7.91%.

Thermally conditioned `BOOB` followed by `BOOBOBBO` five times, with ten
measured runs per process, confirmed the layout change. Default Gentle improved
from 99.187 to 97.657 ms/run (1.54%); checkpoint-enabled Gentle improved from
103.819 to 101.919 ms/run (1.83%). Other sidecars remain columnar because this
evidence applies specifically to complete-box access, not to a general retreat
from compact field scans.

## Direct shipout evidence

The direct-emission refactor removed fresh-path `FrozenShipoutNode` snapshots,
ordinary `PageNode` materialization, and the artifact-byte DVI reparse. A
200-run default capture reduced the complete `shipout_node` subtree from
15.22% to 11.14% of whole-run samples (26.8% relative). Within the candidate,
fused arena emission was 6.47% of the whole run and mutable normalization was
2.58%. The capture still charged 1.05% to a decoded direction prescan; the
accepted implementation subsequently replaced that work with a raw compact-tag
predicate.

Conditioned `BOOB` plus `BOOBOBBO` five times measured 111.963 to 100.468
ms/run by raw default mean. Removing one isolated 258 ms baseline outlier gives
104.277 to 99.315 ms/run (4.76%). A post-tag-optimization checkpoint-enabled
repetition measured 135.664 to 131.800 ms/run (2.85%), with effectively equal
medians of 105.909 and 105.865 ms/run. Host contention produced large paired
outliers, so retain both raw means and medians when comparing this run. All
processes emitted 97 pages and 263,424 DVI bytes; checkpoint runs published
1,108 checkpoints.

## Incremental boundary and paragraph roofline evidence

The 2026-07-17 incremental audit found that accepted canonical boundary
identities were materialized lazily. Every schedule-aligned comparison whose
old record lacked an identity cloned the accepted `Universe`, rolled the clone
back to that boundary, projected the identity, and cached it. A 66-candidate
interaction edit therefore spent 174.5 ms without paragraph recording and
186.9 ms with paragraph recording in candidate validation.

Commit `b2fbbb84` captures canonical identities while accepted boundaries are
live and compares the retained values directly. The same 66-candidate stage
fell to 6 us and 5 us respectively. End-to-end interaction latency fell from
263.7 to 93.5 ms without paragraph recording and from 285.7 to 100.4 ms with
it. The one-candidate height/page-preserving edit remained about 72 ms. The
cost moved to live publication: initial priming and a fully nonconvergent slow
edit each increased by about 22--24 ms. Focused state/executor/incremental
tests, the explicit 1,000-edit convergence test, and `scripts/check.sh` passed.

A separate optimized cold Gentle sampling run measured 100 compilations at
210.252 ms mean with 21,466 main-thread samples. Direct stack attribution was
50.6 ms (24.1%) for paragraph hlist/front-end work and 30.2 ms (14.4%) for
paragraph finalization and line breaking. Page/output work was 64.9 ms
(30.9%), but only 0.8 ms belonged to page-break selection; 3.7 ms was other
output work and 60.4 ms was shipout, page lowering, and artifact construction.
The remaining 64.6 ms was other execution and driver work.

Perfect zero-cost finished-line replay while rebuilding pages therefore has a
129.5 ms whole-document floor, or about a 1.62x ceiling. Hlist-only replay has
a 159.7 ms floor, or about a 1.32x ceiling. These are absolute rooflines rather
than expected edit results because the changed paragraph, restart prefix,
validation, import, and misses remain.

The corresponding design decision is recorded in
[`incremental_memoization.md`](incremental_memoization.md): full canonical
boundary identity remains the fast suffix splice, while the slow path uses an
ordered accepted-history paragraph cursor with per-record validation. It does
not add a reverse paragraph-suffix hash. Direct page/shipout artifact patching
is deferred to later measured work.

The completed accepted-history implementation was measured in an optimized,
uninstrumented eight-pair AB/BA run after deleting the old lookup/admission
stack and retaining only replayable records. Each pagination-changing edit
replayed 246 paragraphs as finished lines, skipped 11,343 commands, imported
993,960 bytes, and had 15 validation misses with no import failures or hlist
fallbacks. Every edit kept the memo-disabled and memo-enabled boundary
schedules equivalent and produced DVI byte-identical to cold.

The result is nevertheless a negative default-enablement decision. The first
final eight-pair run lost 36.718 ms across the two slow edits and 60.633 ms
including initial history publication. After the import optimization below, a
four-pair confirmation lost 35.800/58.240 ms; a second eight-pair run, with a
large fast-edit outlier, lost 45.193/70.547 ms. Interaction deltas were
+1.473, +3.085, and +2.675 ms respectively. The independent fast suffix-splice
median remained flat (within about 1 ms) and still retained 14 pages,
re-shipped 3, and adopted 83. Accepted paragraph metadata was 2,303,932 bytes,
down from 12,937,867 bytes when barriered records were retained.

A ten-run Samply capture attributed only 0.68% of whole-run samples to
`try_reuse_aligned_paragraph`; cursor lookup and prepared input transition were
effectively invisible. Retained-result import dominated that small subtree;
36% of its samples recomputed SHA-256 node identities. Imported graphs now
preserve their already-sealed semantic identities while rebasing nonsemantic
handles, reducing reported import time from roughly 4.4--5.0 ms to 1.26--1.30
ms per slow edit. There was no hidden repeated rollback, linear suffix scan,
or replay-side quadratic responsible for the loss. The limiting factor
appeared to be coverage: only 261 of 889 observed paragraphs were replayable,
while 628 hit barriers. Barrier
telemetry reported 594 group transitions, 174 input transitions, 210
unsupported writes, 50 display-math crossings, and 42 output-routine
crossings, with overlap. The initial equal-depth experiment caused a cold-DVI
mismatch on the inverse edit. Later threshold isolation showed that the group
rule was not the cause: recording had started at a preceding `\bigskip`, while
retained paragraph installation recreated only `\parskip` and the hlist.
Replay therefore silently omitted the outer vertical glue.

The correction treats vertical recording as provisional and discards it after
every delivered command that remains in outer vertical mode. The paragraph
region therefore begins only at the command that actually enters horizontal
mode. Input transitions also recognize a scanner's backed-up first source
token, using its rooted start anchor while still validating the complete raw
byte range. A focused `\vskip` regression and the four-edit Gentle matrix now
prove that vertical material is executed cold and DVI remains byte-identical.

Count/integer mutation accounting was independently simplified. Setters only
invalidate a lazy complete-state aHash fingerprint. Paragraph exit derives a
compact root-survivor redo from the already-compacted environment journal, so
balanced local writes require no record and depth-zero group transitions may
replay safely. Exact incoming fingerprints are the common path; mismatches
validate the survivor cells' entry values before replay. Nonzero groups with
surviving writes remain barriers because final values do not encode assignment
ownership.

The corrected boundary deliberately reduces coverage. On the slow Gentle
edits, 132 regions replay as finished lines, 525 recorded regions hit barriers,
42,183 commands are skipped, 3,735,160 bytes of retained nodes are imported,
and accepted paragraph metadata is 3,916,504 bytes. The barrier counts are 420
group transitions, 69 unsupported writes, 34 input transitions, 20 display
math crossings, and 13 output crossings. Macro-generated paragraph starts
whose vertical setup exhausts the clean root-source alignment are left cold;
recovering them would require late alignment plus an explicit identity for the
already-built horizontal prefix.

A six-run optimized AB/BA confirmation kept every revision byte-identical to
cold and reported candidate-minus-baseline means of +4.213 ms for the two slow
edits, +1.347 ms for the interaction edit, and -0.084 ms for the independent
fast edit. The corresponding medians were +13.835, +3.364, and +1.217 ms. One
large memo-disabled priming outlier made the mean priming-inclusive delta
+6.247 ms versus a +37.920 ms median, so this short run is correctness and
directional performance evidence rather than a new enablement baseline. The
post-audit six-run confirmation, after direct-source pending-token tightening
and cheap provisional-checkpoint abandonment, reported +18.743 ms slow,
+6.337 ms interaction, and +4.568 ms fast means; its priming-inclusive slow
mean was +52.266 ms. Coverage and exact output were unchanged. The spread
between short balanced runs reinforces the release decision: the paragraph
layer remains default-disabled, and no speed win is claimed.

The adversarial implementation review found no repeated accepted-substrate
rollback, global candidate search, suffix scan, or quadratic on the measured
replay hot path. It did find two cleanup issues: barriered records performed
work and retained memory despite never being candidates, and provenance recipe
construction searched the token-origin sequence once per output origin. The
former was removed before the measurements above; the latter now builds one
linear-time aHash ordinal index. With paragraph replay default-disabled, no P1
correctness, asymptotic, architectural, or measurement issue remains in the
released fast suffix path. The unresolved slow-path limitation is explicit
capability coverage, not a hidden cache layer.

### Shared finished-line mount experiment

Issue `umber2-q02h.58` tested the remaining ownership hypothesis without
changing paragraph installation or the page builder. Survivor semantic
payloads are now immutable `Arc` values shared by related Universes. The
restarted store validates and mounts the accepted `NodeListId`, supplies a
local provenance overlay, and restores the retained hlist glue closure. The
ordinary reused-paragraph epilogue still appends each mounted top-level line,
migrating node, and penalty through the existing vertical/page path.

The required before Samply capture was
`/tmp/umber2-q02h58-before.json.gz` (10 balanced incremental pairs, 36,346
weighted samples). `try_reuse_aligned_paragraph` owned 391 samples (1.08%).
Recursive origin rebinding owned 152 samples (0.42%), including 90 SHA-256
compression samples, and retained-list cloning owned 78 samples (0.21%). The
optimized before timing run reported +23.477 ms combined slow-edit
enabled-minus-disabled and +7.212/+4.765 ms executor deltas.

The matched after capture `/tmp/umber2-q02h58-after.json.gz` had 38,017
weighted samples. Replay fell to 120 samples (0.32%); neither origin rebinding
nor retained-list cloning appeared in sampled stacks, and the complete mount
subtree was 24 samples (0.06%). SHA-256 under replay fell from 104 samples
(0.29% whole-run) to 4 (0.01%), with none attributed to origin refreezing.

The separate optimized, profiling-stat ten-pair run reported 132 line hits and
zero imported semantic bytes on each slow edit. Enabled and disabled survivor
source-word promotion was effectively identical (192,196 versus 192,198 on
each slow edit), so line hits no longer add promotion volume. The slow-path
delta was -8.219 ms mean/-6.196 ms median; executor deltas were -11.195 and
-9.716 ms. Interaction was +0.876 ms mean, fast was -0.191 ms mean/+0.842 ms
median, and slow-plus-priming remained +25.987 ms mean/+26.489 ms median.
Every revision preserved the disabled named-boundary schedule and cold DVI
bytes. These results validate the mount seam but do not enable paragraph
recording by default because accepted-history publication still outweighs the
two-edit slow-path win over a complete session.

A final two-pair telemetry pass separated recycling releases from O(1) shared
payload drops. Each slow edit recycled 1,161 roots with memoization enabled
versus 1,162 disabled, while 1,836--1,894 local roots were released by dropping
their shared payload reference. Together with the unchanged source-word count,
this confirms that finished-line hits do not hide promotion or recycling
amplification behind the mount.

### Output-provenance closure experiment

Issue `umber2-q02h.59` removed the expanded-token vector and its parallel
stable-root trace from paragraph recording. Cold publication now walks only
the retained hlist and finished-line graphs. Its recipe stores one full stable
anchor per editor piece, compact `(piece ordinal, start, end)` output ranges,
and depth-first `u32` origin slots. Replay allocates current origins only for
those distinct output ranges and mounts them through the q02h.58 survivor
overlay, so ordinary node traversal, page building, output routines,
diagnostics, and shipout all see the same current-revision provenance.

The focused 4,096-expanded-`\relax` regression retains at most three roots and
three slots for its two output characters. A finished-line replay regression
resolves the mounted origin in the current layout and then as typed `Deleted`
after its fragment is replaced. This directly guards the required asymptotic:
expanded tokens which produce no accepted output contribute no provenance
metadata and cause no replay-time origin allocation.

The same optimized Gentle matrix retained 1,999,076 bytes of accepted
paragraph metadata versus 3,916,504 bytes on the q02h.58 baseline (-48.96%).
Each slow edit still mounted 132 finished-line hits, skipped 42,183 commands,
imported zero semantic bytes, preserved the disabled boundary schedule, and
emitted cold-identical 100-page/279,176-byte DVI. The instrumented telemetry
pass reported 0.65--0.75 ms total paragraph import/mount time for each 132-hit
slow edit.

The final ten-pair optimized AB/BA run encountered severe host contention
(individual paired samples ranged from -595 to +975 ms), so medians are the
responsible latency summary. Paragraph-enabled minus disabled medians were
+1.366 ms for the combined slow path, +23.753 ms with priming, +2.032 ms for
interaction, and +1.832 ms for the independent fast path. Disabled/enabled
priming medians were 267.313/282.726 ms. The result does not change the
default-disabled release decision; it records the closure's large metadata
win, sub-millisecond-per-edit replay provenance cost, and exact-output guard
under a noisy timing environment rather than claiming an end-to-end win.
