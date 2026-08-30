# Resident `scan_toks` collector

Bead: `umber2-66p0.8.40.55`

## Named residual

The authenticated `.8.40.54` 50M capture identifies
`CommandProcessor::scan_toks_buffers` at 54 samples, 646,813,298 weighted self
cycles, 1.51% self, and 14.57% inclusive. Macro-definition scanning accounts
for 0.99 profile points of its self ancestry. At base `c88780143`, the
profiling assembly has three `scan_toks_buffers` monomorphizations of 34,490,
34,490, and 34,612 bytes plus three 666-byte `finish_scan_toks_sink`
monomorphizations. Source routes every append and both completion phases
through `ScanToksSinks`, `ScanToksSink`, and `ScannedToksPart`.

## Resident ownership

One move-only `ScanToksCollector` now reserves either the final two attempt
token branches or one definition builder before the scanner scope opens. It
retains the one active writer and a monotonic parameter, replacement, or
complete phase. Parameter scanning transitions that writer in place;
replacement collection, direct splices, whole-line `read_toks`, suspension,
rollback, and final sealing borrow or retain the same owner. Completion returns
only the already-resident typed coordinates. The three old route/part types,
the phase-carried parameter result, and the replacement-progress output route
are absent.

Accepted packed words go through one append boundary into final attempt or
definition storage, where the lane length and provenance-bearing word are
updated together. Direct `\the`, `\unexpanded`, and `\detokenize` paths now
stream into that boundary instead of materializing temporary token vectors.
Only observation publication and selected runaway diagnostics traverse sealed
words. Failure truncates the collector's exact opening mark; a resource
suspension parks the collector itself in the existing typed scanner frame.

The current profiling assembly names `collect_replacement` explicitly at
21,413 bytes and shrinks each outer `scan_toks_buffers` monomorphization to
18,723, 18,723, and 18,764 bytes. Its only collector callees are the 402-byte
append, 509-byte parameter transition, and 550-byte final settlement. The old
`finish_scan_toks_sink` symbol and all old source route names are absent. This
is structural assembly evidence, not a new runtime profile claim.

## Focused evidence

The profiling-only mixed gate runs one and 4,096 rounds. Each round performs
an unexpanded general scan, expanded general scan, macro definition, and
write-like owned scan after warming and rolling back the same high water. Its
exact counter vectors are:

| Rounds | Collectors | Appends | Fact updates | Phase transitions | Settlements |
| -----: | ---------: | ------: | -----------: | ----------------: | ----------: |
|      1 |          4 |       5 |            5 |                 4 |           4 |
|  4,096 |     16,384 |  20,480 |       20,480 |            16,384 |      16,384 |

At both scales duplicate phase dispatches, fact rescans, whole token-list
copies, whole command copies, and whole frame copies are exactly zero.
Delivery-and-scan and attempt-scratch allocation deltas are independently zero
calls and zero requested bytes. The gate measures only after input reservation;
the warm rollback retains token chunks and definition builders for exact reuse.

The existing macro-definition, `edef`/`xdef`, token assignment,
write/general-owner, grouping, parameter-marker, outer/noexpand, suspension,
publication-collision, rollback, runaway, and durable-definition tests remain
the semantic regression authority. The crate boundary test additionally
rejects every retired route type and any second ordinary expansion loop.
