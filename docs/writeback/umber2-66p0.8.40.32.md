# `umber2-66p0.8.40.32`: resident cold-operation scan

## Attribution

The authoritative fixed-clock census attributed the remaining cold-scan
carrier to 1,091,251 public copy calls and 165,985,176 bytes. The carrier was
the by-value `Result<ColdOperation>` returned by scalar, structured, alignment,
recovery, arithmetic, and leader helpers before the completed leaf was
installed in the caller-owned `ColdOperationSlot`.

Cold helpers now borrow that singular slot, write their semantic leaf at its
final address, and return only `Result<()>` or the leader scanner's compact
boolean. The destination macro expands at the terminal construction site; it
does not pass the 264-byte leaf through a helper argument. The only remaining
`ColdOperationSlot::write` call is `OperationFrame::write_unavailable`, where a
genuine unavailable-resource handoff moves the completed operation into its
typed suspension owner.

This is an ownership-only change. The existing scan and recovery matrix and
its cited TeX82 behavior remain unchanged, including §§1045--1054 terminal
handling, §1090 mode recovery, §§1210--1214 arithmetic assignments, alignment
dispatch, rollback, diagnostics, and resource resume.

## Focused evidence

The optimized A/B uses an archive of exact base `c910f6e33` and the candidate,
the same 1,000-call direct production episode, and seven `perf stat`
repetitions. Both rows produce the same work vector
`(67073,65073,46019,20025)`, output sizes, 9,183 allocation calls, and
5,455,982 allocated bytes.

| Evidence                  |                 Base |            Candidate |                 Delta |
| ------------------------- | -------------------: | -------------------: | --------------------: |
| `memcpy` calls / bytes    | 132,569 / 14,069,768 | 122,560 / 12,539,075 |  -10,009 / -1,530,693 |
| `memmove` calls / bytes   |      2,009 / 336,296 |      2,009 / 336,296 |                 0 / 0 |
| 152-byte `memcpy` carrier |   12,015 / 1,826,280 |      2,005 / 304,760 |  -10,010 / -1,521,520 |
| 264-byte `memcpy`         |            4 / 1,056 |            4 / 1,056 |                 0 / 0 |
| cycles                    |          811,341,929 |          770,536,865 | -40,805,064 (-5.029%) |
| instructions              |        1,186,624,946 |        1,186,312,837 |    -312,109 (-0.026%) |

The targeted 152-byte row disappears without shifting to the 264-byte leaf or
to `memmove`. The focused in-process one/4,096-cycle test separately reports
zero `DeliveryAndScan` allocation calls and bytes, zero frame/slot address
changes, and zero overlapping moves after warmup. Raw build, copy, output, and
counter evidence is under `target/umber2-66p0.8.40.32/`.

## Verification

- `one_and_4096_cold_scan_cycles_are_allocation_free_and_stationary`
- `fused_hot_and_typed_cold_dispatch_share_one_interpreter`
- all 372 `tex-command` test targets
- the canonical command-stream tracer
- `scripts/check.sh`

The `tex-exec` target passes 733 of 737 tests. The four failures are the
alignment-preamble span tests asserting at `tex-command/src/processor/expand.rs:683`.
The representative
`preamble_span_expands_one_token_and_preserves_later_template_meaning` failure
was rebuilt and rerun from the untouched `c910f6e33` archive and fails at the
same assertion, so this is an exact-base defect rather than a regression in
the cold-operation change.
