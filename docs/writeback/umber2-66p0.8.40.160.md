# `umber2-66p0.8.40.160`: minimal resident advance

## Adopted boundary

Replay, attempt, and durable rows retain their distinct lifetime owners and
one shared `TokenRowHeader`. Admission proves the complete storage extent.
Ordinary delivery checks the header's exhaustion boundary, performs exactly
one storage-specific safe packed-word read, advances the header position once,
and enters the one existing destination-owned command admission and settlement
tail.

Attempt-local rows now consume their admission proof directly: the hot read no
longer authenticates the attempt key and token-list row or reconstructs a
general list view for each word. Replay's physical cursor remains independent
because rollback must restore its prefix/run/segment coordinate together with
the logical frame position. Prefix-to-body and fixed-segment changes are
out-of-line boundary transitions; the ordinary replay word reads its current
segment and advances that physical cursor. A successful safe storage read is
the proof used by the frame's branch-free resident advance, so delivery does
not repeat the limit decision.

Parameter substitution remains the macro-body boundary, malformed parameter
replay still fails through the existing cold interception, and exhausted rows
still retire through the exact replay/attempt/durable owner. Source delivery,
suspension ownership, replay completion, fuel charging, rollback journals,
provenance, and the caller-owned command destination are unchanged. The change
adds no cache, duplicate representation, alternate command path, unsafe alias,
or eager copy.

## Focused comparison

The assigned base `abe425f51` and candidate used the same release/profiling
build and the existing `fused_raw_expanded_delivery` exact vector. Each run
performed 1,000,000 raw and 1,000,000 expanded deliveries split across 666,667
replay, 666,666 attempt, and 666,667 durable words. Both reported exactly
2,000,000 fuel charges, frame steps, and meaning lookups; 1,000,000 expanded
completions; zero relays, command copies, warm allocations, and requested
bytes.

| Measure                    |          Base |   Candidate |                Delta |
| -------------------------- | ------------: | ----------: | -------------------: |
| User instructions          | 1,036,261,783 | 990,951,620 | -45,310,163 (-4.37%) |
| User cycles                |   442,896,726 | 409,722,283 | -33,174,443 (-7.49%) |
| Resident-transition symbol |       4,905 B |     4,637 B |      -268 B (-5.46%) |
| Warm allocations/bytes     |         0 / 0 |       0 / 0 |            unchanged |
| Semantic copies/relays     |         0 / 0 |       0 / 0 |            unchanged |

Candidate counters are the mean of three `perf stat` runs; instructions varied
by less than 0.01% and cycles by 2.59%. The candidate's independently
interposed public-copy receipt reports 133 `memcpy` calls totaling 24,296,166
bytes and two zero-byte `memmove` calls, all outside the measured resident loop,
with zero table collisions, overflow, or probe-internal calls. Its exact binary
SHA-256 is
`ea910a1baa4d9073bfdffdd29d0e1f37cc1f7323f2ebbf1cbcdb9dbfd846f43a`.

## Validation

- `cargo test -q --tests -p tex-command`: 391 unit tests and 23 boundary tests pass.
- `fused_raw_expanded_delivery`: exact semantic counts and zero warm allocation pass.
- `scripts/check.sh`: all four gates pass; both clippy resolutions cover 32 workspace members.
