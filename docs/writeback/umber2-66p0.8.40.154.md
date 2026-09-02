# `umber2-66p0.8.40.154`: one resident token-row header

## Result

Replay, attempt, and durable input variants now embed one
`TokenRowHeader`. The header is the sole owner of the packed frame and its
logical position/limit, rollback marker, behavior, retirement policy, trace,
and inherited source context. Each concrete variant performs only its distinct
word read; all three enter the same source-defined first-touch, advance,
parameter-interception, exhaustion, and existing command-admission transition.

Replay retains only its genuinely storage-specific run/segment cursor. Its
duplicated logical position was deleted, and the first-touch inverse swaps that
physical coordinate together with the header position. Macro-body,
macro-argument, and source rows and their lifetime owners are unchanged. No
carrier, cache, second loop, dynamic dispatch, command-family path, or alternate
row representation was added.

The layout gate remains exact: `InputLevel<()>` is 88 bytes. The new common
header is 48 bytes, replay/durable/attempt rows are 72/80/72 bytes, and the
packed frame's existing limit replaces the duplicated stored-row length.

## Focused evidence

The exact base `eb7d61d91` and candidate were built independently in release
mode and ran the existing `warmed_mixed_stored_cursor` row once each. Both
delivered 5,000,000 commands, retired 1,250,000 rows, performed one rollback,
produced checksum 8,455,000,000, and reported zero measured allocations and
zero requested bytes.

| Measure                    |            Base |       Candidate |                      Change |
| -------------------------- | --------------: | --------------: | --------------------------: |
| Instructions               |     383,781,574 |     383,781,074 |             -500 (-0.0001%) |
| Whole-binary text          |     2,118,961 B |     2,116,644 B |           -2,317 B (-0.11%) |
| Resident-transition symbol |         4,823 B |         4,880 B |                       +57 B |
| `memcpy`                   | 112 / 327,522 B | 112 / 327,481 B |             0 calls / -41 B |
| `memmove`                  |         2 / 0 B |         2 / 0 B |                   unchanged |
| Timed row                  |   10.99 ns/call |   11.70 ns/call | diagnostic single-run noise |

The copy probe reported zero overflow for both APIs. Across the two resident
source files, the refactor deletes 381 lines and adds 213, a net deletion of
168 lines before documentation and tests. The focused result is therefore
instruction- and copy-neutral while reducing total generated text and deleting
the three replicated semantic state machines; the small inlined symbol increase
does not create another owner or transition.

## Validation

- `cargo test -q -p tex-command --tests`: 391 unit tests and 23 integration
  tests pass, covering provenance, parameter replay, recovery, suspension,
  exhaustion, retirement, and checkpoint rollback/redo.
- The mixed-storage gate preserves zero warm allocations and exact semantic,
  retirement, and rollback counts.
- `scripts/check.sh`: biome, rustfmt, and both clippy resolutions pass across 32
  members; the sole dprint table-alignment failure was formatted and its named
  gate rerun successfully.
