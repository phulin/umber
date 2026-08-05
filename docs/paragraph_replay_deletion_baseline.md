# Paragraph replay deletion baseline

This is the reproducible pre-deletion receipt for `umber2-9hid.8`. The parent
decision is to delete paragraph input transaction replay completely. These
measurements quantify the tradeoff and do not reopen that decision.

## Identity

- Source: `13eaa9afbad1daab3f83848f67278b735353c01b`, based on
  `d4a95ab357854263d8630ee5e55c31ff526df30f`.
- Binary: `target/profiling/gentle-profile`, 322,242,656 bytes, SHA-256
  `c1742601a806a456a031f912d65093405126e0b15d763e1eb3fe42152561a761`.
- Build: `cargo build --profile profiling -p umber --bin gentle-profile
  --features profiling-runner,profiling`; `Cargo.lock` SHA-256
  `610aeafd0e237dab5ba4bd61ca7157d24c6fb66763cb88b7853be09e9487182c`.
- Toolchain: `rustc 1.93.0 (254b59607 2026-01-19)`, target
  `x86_64-unknown-linux-gnu`.
- Host: Linux 6.8.0-107-generic x86_64, UTC, measured on 2026-08-05.
- Distribution: none. The runner uses a memory-backed Plain TeX input set,
  not a TeX Live manifest. `plain.tex` is SHA-256
  `7c1223b880fa513542a616b86c4fa02d5ee703839427951982d7e311fd9049b2`;
  `hyphen.tex` is
  `2c18acdc04c1a066aeb1759905e7ca449f0616c314b5ed6aebe55b9d4a89b8d4`.
  The unavailable representative Gentle input is SHA-256
  `b3e1acf4a6feb6a13b0ace279a5be7f1546ac2198452188f6fb22b4eb590ba3f`.

The binary is an attributed optimized build. Profiling atomics are included in
latency, so compare it only with a post-deletion binary built with the same
features. `/usr/bin/time` wrapped the already-built binary, never Cargo.

## Workloads

The committed pairs live in
[`benchmarks/paragraph-replay/workloads`](../benchmarks/paragraph-replay/workloads).
Every measured advance alternates before to after and after to before in one
retained session, then compares DVI bytes with a fresh execution of the target
source. The runner generates `long` as 384 copies of its pinned paragraph text
and changes the first `Alpha` to `Omega`. The synthetic stabilization workload
runs 16 unchanged-root external-input-delta passes, alternating an unused
generated macro definition. Its root contains 16 paragraphs, four of which
also consume the unchanged generated width macro.

Pair identities, before then after, are:

| Workload     | Before SHA-256                                                     | After SHA-256                                                      |
| ------------ | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| unchanged    | `da664f1c4a19b0fa8a3234ecb8cd3703ee09200e28add6e0093800313497ca77` | `83d793c5f2513a7a9b2c4fa1b3c8bf88917fc5cce40247851270064c1990e119` |
| prefix       | `8b2fe44fe13a1112cc9b8613cedcd677a0aa5b5346aab5bf0143d8c8c4a80efd` | `899c9ee43cab7f5090c3f342c2f17a92635532c861ebd716b64adf031f629537` |
| suffix       | `6767a6324daa73db3739a8d8767d4648a5f4ca5acb18441d4131ce59fd0fb6d7` | `ae45755ef305f8e1afcb4b5a162572d79f73ec0bfc51e0ef6501543b3e78a29a` |
| display-math | `a13520e2a23c07f307b0808eb9d12129d18a063697b0702d4a1a8c49c2ae2750` | `e54237617a0627327ce974997f1f7692e87f80c9a631deb5cdc47c411b541b14` |
| macro        | `b81f79c109e9ec49943c51e74f863009ff95bed08d71143a581f96c35925bff6` | `fd11e792dfa0bad98d75292c8af8635dd4f351c73d0d50e80316609c4942b5b2` |
| conditional  | `2f4f669309a583ff0ef22b81dee7818164dee92f250d16fb978eee5eb026f554` | `676f58c40fb5cc115feba183e47ed54054f365c180163220341245ba4b426434` |

Run one pair with:

```bash
/usr/bin/time -f 'wall_s=%e maxrss_kib=%M' \
  target/profiling/gentle-profile \
  --iterations 6 --warmups 2 \
  --paragraph-workload prefix --memo-layers paragraph
```

Replace `prefix` with `unchanged`, `suffix`, `display-math`, `macro`,
`conditional`, or `long`. Use two iterations and one warm-up for `long`. Run
the generated-input case with:

```bash
/usr/bin/time -f 'wall_s=%e maxrss_kib=%M' \
  target/profiling/gentle-profile \
  --iterations 2 --warmups 1 \
  --synthetic-stabilization-replay --memo-layers paragraph
```

## Pre-deletion results

The small pairs used six measured advances after two warm-ups. Fresh columns
are the two one-off cold executions used for byte identity. RSS is the maximum
for the complete process. Times are milliseconds.

| Workload     |  Fresh before/after |    Edit mean/median |        Edit min/max |   RSS KiB | Last reexecuted paragraphs/bytes/commands |
| ------------ | ------------------: | ------------------: | ------------------: | --------: | ----------------------------------------: |
| unchanged    |     185.119/172.539 |     167.298/166.612 |     160.884/177.610 |    32,780 |                               5/534/2,050 |
| prefix       |     280.057/307.237 |     211.775/220.547 |     165.579/274.253 |    33,464 |                               5/524/2,073 |
| suffix       |     175.639/170.946 |      86.052/164.031 |       5.597/167.714 |    32,636 |                               5/542/2,091 |
| display-math |     157.169/157.158 |     160.685/162.116 |     157.839/165.235 |    27,276 |                               2/375/1,915 |
| macro        |     163.119/160.395 |     164.740/165.886 |     162.157/167.236 |    29,296 |                               3/395/1,912 |
| conditional  |     164.599/158.097 |     164.581/165.803 |     162.402/167.035 |    30,452 |                               2/388/1,807 |
| long         | 2,897.228/3,384.592 | 4,999.019/6,510.617 | 3,487.421/6,510.617 | 1,249,932 |                       338/100,271/103,872 |

All results were byte-identical to their cold target. No small pair attempted
paragraph replay on its final direction. The suffix pair is the current best
incremental case: alternating directions produced the bimodal 5.597 ms to
167.714 ms range through checkpoint convergence. The long prefix edit is the
current worst case: it had 337 paragraph validation misses, no hits, retyped
57 pages, and reexecuted 338 paragraphs. This is intentionally evidence about
the implementation being deleted, not an activation recommendation.

The synthetic generated-input comparison used two AB/BA sessions per policy,
16 passes per session. Cold initial execution averaged 222.025 ms; replay
initial execution averaged 209.698 ms. Cold passes averaged 216.172 ms and
replay passes 214.764 ms. The paired complete-session replay-minus-cold delta
averaged -34.861 ms but ranged from -95.513 ms to +25.790 ms. Replay recorded
208 validation misses, no lookups or hits, reexecuted 44,288 bytes, retained
832 history-metadata bytes, and peaked at 50,088 KiB RSS. This workload is the
unchanged-root generated-input stabilization worst case on this base.

## Snapshot and continuation ownership

`StepSnapshot` has a fixed logical size of 8,232 bytes in this binary. The
profiling counter times its complete capture and separately charges the
shallow `ParagraphRecorder` plus dynamically cloned deque/vector elements.
It does not charge payloads behind shared `Arc` roots, so the byte count is a
lower bound on owned state touched and not a heap-retention measurement.

Across the long workload, 519,605 step snapshots consumed 5.291 seconds of
capture time and 4,277,388,360 logical bytes. Of those, 509,475 snapshots were
inside active paragraphs; they cloned 173,548,440 logical recorder bytes, or
340.64 bytes per active command. Continuation ownership detached 674 paragraph
endpoints in 2.523 ms and materialized them in 1.336 ms. The same run's four
fresh/edit executions peaked at 1,249,932 KiB RSS.

For scale, the unchanged small pair produced 20,990 snapshots in 231.612 ms;
4,540 were active-paragraph snapshots and cloned exactly 312 logical recorder
bytes each. It detached/materialized 20 endpoints in 0.097/0.320 ms. The
synthetic stabilization comparison detached and materialized 1,248 endpoints
across 96 calls in 3.550/7.935 ms.

Valgrind Memcheck supplied allocation volume for one prefix iteration plus one
warm-up and its two cold references. Paragraph recording allocated 347,065,881
bytes in 1,557,607 allocations; the same `PureMemoRuntime` with every recording
layer off allocated 347,063,176 bytes in 1,557,577 allocations. The marginal
observed volume was 2,705 bytes and 30 allocations. Both runs reported the same
one unrelated uninitialized-value context, so this is allocation accounting,
not a Memcheck correctness receipt.

## Representative corpus limitation

The committed `gentle-profile --cold-memo-layers`, `--incremental-path`, and
`--stabilization-replay` Gentle modes panic on the assigned base at
`tex-state/src/dependency.rs:302` with `paragraph break region already active`.
Issue `umber2-vf1m`, linked to the parent epic, owns that defect. This issue did
not change paragraph semantics to bypass it. The 384-paragraph generated
workload supplies the reproducible long-document baseline; the exact Gentle
hash above preserves the intended corpus identity for a later rerun.

## Post-deletion comparison

After paragraph replay is removed, rerun every command above with the same
toolchain, features, order, iterations, and workload identities. Compare:

- DVI bytes for every target against fresh execution;
- fresh and edit mean, median, minimum, and maximum latency;
- accepted convergence boundary, retained/retyped/reused pages, reexecuted
  bytes, tokens, commands, and paragraphs;
- accepted-output snapshot, fork, executor, detach/materialize, and DVI
  materialization time;
- `StepSnapshot` size/capture time and active-paragraph cloned bytes;
- Valgrind total allocation count/bytes and `/usr/bin/time` maximum RSS; and
- generated-input pass latency, retained history bytes, and validation work.

Expected structural differences are zero paragraph transaction detach and
materialize work, no active `ParagraphRecorder` clone charge, and reexecution
from the nearest accepted `CommandSummary`. Correctness still requires exact
cold output and boundary-schedule comparisons. Report observed performance
deltas without treating any regression as grounds to retain or revive replay.
