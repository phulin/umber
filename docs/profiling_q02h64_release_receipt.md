# Paragraph replay q02h.64 release receipt

Status: final default-enablement evaluation, 2026-07-17.

## Identity and method

The evaluated tree was commit `354c1d0730cf3f4ea98ce2493347e210be2ccc46`
on `arm64` macOS 15.6.1, using Rust/Cargo 1.93.0. The corpus was the pinned
local `third_party/corpus/gentle.tex`. The release comparison used the
uninstrumented profiling build, ten balanced AB/BA pairs, two in-process
warm-ups, and five accepted edits per session:

```bash
cargo run --profile profiling -q -p umber --bin gentle-profile -- \
  --repo-root /Users/phulin/Documents/Projects/umber2/.worktrees/umber2-vfqs \
  --incremental-edit --iterations 20 --warmups 2
```

The runner alternated disabled/enabled order for even samples and
enabled/disabled order for odd samples. It compiled each revision cold after
the pair and rejected unequal DVI, boundary schedules, fast-path page
accounting, or absent mounted-hlist hits. `profiling_stats=false` was printed
by the release run. A separate four-pair `--features profiling-stats` run
supplied named counters; it was not used for the release latency decision.

The sampled attribution command was:

```bash
GENTLE_PROFILE_ITERATIONS=12 GENTLE_PROFILE_WARMUPS=1 \
GENTLE_PROFILE_OUTPUT=target/profiles/q02h64-final-incremental.json.gz \
scripts/profile-gentle.sh --incremental-edit
```

The local capture SHA-256 was
`dbd94dcb61f7e23e9b7b699a0dab22fba4c34979b1eb8ea957467b05ad71a1da`;
its presymbolication sidecar was
`7a229aeb09e8234166390ac244fd850a9c9ad5ed2ed39d7d023a3782c179f1a4`.
Large captures remain ignored. This compact receipt, the exact commands, and
the aggregate results are durable; issue `umber2-q02h.57` remains the general
raw-per-sample receipt-format task.

## Release timing

All deltas below are paragraph-enabled minus paragraph-disabled. Edit-path
means and medians are paired; individual policy means are shown where they aid
review. Priming has a paired mean, but the runner reports policy medians rather
than a median of the paired priming differences.

| Path                                    | Disabled/enabled mean | Paired mean |                                    Paired median |               Range |
| --------------------------------------- | --------------------: | ----------: | -----------------------------------------------: | ------------------: |
| cold priming                            |    245.025/281.462 ms |  +36.437 ms | not reported (+33.616 ms between policy medians) | 233.014--298.390 ms |
| slow: large insertion                   |    256.576/254.102 ms |   -2.474 ms |                                        -4.476 ms | -28.601--+35.765 ms |
| slow: inverse removal                   |    256.706/249.003 ms |   -7.703 ms |                                        -6.534 ms | -25.561--+11.779 ms |
| combined slow                           |                    -- |  -10.177 ms |                                       -10.252 ms | -43.658--+31.926 ms |
| slow plus priming                       |                    -- |  +26.260 ms |                                       +23.219 ms | -19.016--+75.354 ms |
| interaction                             |      98.375/92.330 ms |   -6.045 ms |                                        +0.137 ms | -101.521--+7.493 ms |
| fast suffix adoption                    |      70.806/70.518 ms |   -0.288 ms |                                        +0.243 ms |   -6.544--+4.601 ms |
| hlist rebreak                           |    254.889/256.618 ms |   +1.729 ms |                                        +3.810 ms | -25.006--+22.349 ms |
| complete five-edit history plus priming |                    -- |  +21.657 ms |                                       +25.134 ms | -83.468--+83.480 ms |

The interaction mean contains one 193.249 ms disabled outlier; its paired
median is the responsible direction. The fast path remained flat and retained
14 pages, re-shipped 3, adopted 83 through one subtree, and ran at
0.245x/0.244x cold latency. The rebreak path mounted 132 accepted hlists but
did not establish a latency win.

## Work, coverage, and retention

Both pagination-changing edits mounted 132 finished-line results, skipped
42,183 commands, and encountered 525 barriered regions. The eligible cohort
was therefore 20.09% by region count, but the timing conclusion does not
multiply that count by a uniform paragraph cost: the measured executor deltas
were -9.319 and -12.857 ms. The interaction edit mounted 10 finished-line
results and skipped 2,938 commands. The rebreak edit mounted 132 hlists,
skipped 42,183 commands, and reported 132 `BreakDependency` fallbacks.

The slow/rebreak barrier events were 420 unsupported group transitions, 69
unsupported writes, 34 input transitions, 20 display-math crossings, and 13
output-routine crossings. Reasons overlap and are not a work-weighted
distribution. Current telemetry reports accepted graph/metadata bytes but not
per-paragraph cold duration or barrier-graph size; `umber2-q02h.65` owns that
profiling-only gap so this receipt does not infer savings from uniform counts.

Accepted paragraph history retained 2,091,680 bytes after priming (161
records, 12,991.8 bytes/record), 2,126,680 bytes after each slow edit (132
published records, 16,002.6 bytes/record for that publication), and 1,788,468
bytes after hlist rebreaking (132 records, 15,831.9 publication bytes/record).
Detached-cache retention and evictions were zero. History publication/drop
itself averaged only 0.289, 0.225, and 0.253 ms on the large, inverse, and
rebreak edits; the remaining end-to-end cost includes substrate publication
and acceptance.

The four-pair attributed run confirmed the direction: combined slow replay
was -11.690 ms mean, but priming was +39.308 ms and slow-plus-priming was
+27.618 ms. On the two slow edits, enabled executor work was -5.828 and
-16.463 ms; acceptance added +3.895 and +2.235 ms. Hlist rebreak was +7.264
ms with executor essentially flat (+0.399 ms) and acceptance +4.577 ms.

The Samply capture contained 58,901 weighted main-thread samples. Inclusive
attribution was 379 samples (0.64%) for aligned paragraph replay, 143 (0.24%)
for finished-line publication, 121 (0.21%) for paragraph provenance traversal,
41 (0.07%) for paragraph-result retention, and 12 (0.02%) for accepted-history
transition. This corroborates the stage counters: no replay-side subtree is a
remaining dominant hotspot.

## Correctness, long session, and decision

Every policy/revision pair published the same boundary schedule and matched
its cold 100-page DVI byte for byte: 279,176, 279,248, 279,176, 279,176, and
279,176 bytes. `scripts/test-incremental-fuzz.sh` passed 1,000 accepted edits
with a cold DVI comparison after every revision in 1.06 seconds; the timed test
process peaked at 53,346,304 bytes RSS. The complete workspace test suite,
explicit Story/Gentle/TRIP/e-TRIP parity selection, `scripts/check.sh`, and
`scripts/check-snapshot-budgets.sh` passed on the documented tree.

Paragraph recording remains opt-in. Finished-line replay now wins the
representative pagination-changing slow path, and the fast path has no
material regression, but priming regresses 14.87% by the means and reverses
the slow-path session result to +26.260 ms. Hlist rebreak also remains positive.
These fail the documented non-regressing priming/session gate. Issue
`umber2-q02h.66` is the focused remaining enablement blocker; default
enablement must be reconsidered only from another full path-separated release
evaluation.
