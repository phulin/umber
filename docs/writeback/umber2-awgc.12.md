# umber2-awgc.12: Direct-Prefix Work Contract

## Decision

Keep direct-prefix commit semantics. Do not restore aggregate prefix replay,
and do not synthetically charge command-work counters for transitions the
engine no longer executes.

The 12,000,000-fuel promotion contract is versioned at this transaction
cutover. Fuel charges and raw token-frame steps remain exact against the
historical authority. Semantic events, diagnostics, state, effects, artifacts,
DVI, and PDF remain exact. Expanded deliveries, meaning lookups,
scanner-status tokens, and deferred-write expansions remain reported against
the historical vector as replay-sensitive deltas; every increase and decrease
must be attributed, and focused controls must reject unrelated added work.

## Earliest transition

The exhaustive `tex-command-stream` run is clean with zero ordered semantic or
geometry divergences. The first work transition is commit `18077d0fe`, at the
first resource miss after successful ordinary commands in the same production
episode.

Before the cutover, `execute_aggregate_operation` captured one rollback root
for as many as 256 commands. A later resource miss rolled that whole prefix
back; retry delivered, expanded, scanned, and dispatched it again. After the
cutover, `execute_direct_episode` commits each successful ordinary prefix
before entering the one-operation compatibility adapter for the resource
command. Retry therefore cannot and must not redo the prefix.

This is a host-transaction transition, not a TeX semantic transition. TeX82
§1030 defines main-control delivery but has no resource-suspension rollback
envelope. The project-owned monotonic command ledger records actual work and
is not restored with semantic state. Reproducing the historical secondary
vector would consequently require either undoing direct-prefix commits or
inventing charges for absent work; both are forbidden by this decision.

## Focused accounting control

`production_batch_keeps_ordinary_prefix_on_resource_need` runs
`\count0=11 \input child\end` through the direct production episode and a
negative-control aggregate retry. Both finish with `\count0=11`. Direct retry
has exact work `(24, 24, 21, 6, 0, 0)`; widening the current compatibility
adapter to the historical 256-operation rollback root has
`(32, 32, 30, 8, 0, 0)`. The aggregate form adds exactly eight fuel charges,
eight token-frame steps, nine expanded deliveries, and two meaning lookups;
scanner and write counters are unchanged. The assertion makes eliminated
replay concrete and rejects an unrelated increase.

## Pinned receipt

The guarded row uses the historical source archive, `ArXiv.tex`, schema-11
format, schema-3 distribution, ordered 105-key closure, offline 528-file cache,
120-second timeout, and 1,536-MiB RSS limit. The pinned SHA-256 values remain:

- source archive: `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected source: `816440f61d611fa57cef802e6f372b9337be1cc4e48e5536d4bad1014ec537`;
- format: `32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`;
- distribution: `560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`; and
- closure: `75d85bb12f8fa5eba0ae2a42daf73fd86c44852ecdc230196455b9aea24565b5`.

The typed status is 1, stdout is empty, no PDF or input-record file is
published, no acquisition occurs, and cache inventory is byte-identical before
and after.

| Counter               | Historical | Direct prefix |    Delta | Contract                           |
| --------------------- | ---------: | ------------: | -------: | ---------------------------------- |
| Fuel charges          | 12,000,000 |    12,000,000 |        0 | exact                              |
| Token-frame steps     | 11,999,815 |    11,999,815 |        0 | exact                              |
| Expanded deliveries   |  1,253,905 |     1,217,976 |  -35,929 | attributed replay-sensitive        |
| Meaning lookups       |  3,485,522 |     3,259,209 | -226,313 | attributed replay-sensitive        |
| Scanner-status tokens | 10,639,582 |    10,902,207 | +262,625 | attributed endpoint redistribution |
| Write expansions      |      1,136 |         1,050 |      -86 | attributed replay-sensitive        |

The baseline recorded 186 resource rollbacks and 125,145 delivery/scan
attempts. The direct-prefix row records 140 resource rollbacks and 91,680
delivery/scan attempts. Eliminated aggregate retries move the fixed raw-frame
endpoint forward, which changes the mix of frames: more of the later frames
are under non-normal scanner status even though the total raw-frame count does
not increase. The four secondary fields overlap and are not an additive work
total.

## Validation

The exact guarded receipt, focused accounting control, exhaustive tracer,
routine native suite, and formatting/lint gate are the acceptance set. Local
evidence is under `target/umber2-awgc.12`.
