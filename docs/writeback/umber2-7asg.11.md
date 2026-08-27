# `umber2-7asg.11`: scalar scanner call frames

## Evidence boundary

The paired exact-20M rows use the immutable arXiv `2606.12566` source,
packed distribution root `721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, and authenticated 123-key closure documented by
the combined copy-kernel audit. Both profiling executables were built with
`RUSTFLAGS='-C force-frame-pointers=yes'`. The base executable has SHA-256
`5b39fc8c1eb2c724ad94b0c0dd4d1aaca21dc20beb7888079441f9f3d5cf6f20`;
the candidate has SHA-256
`ff5fd0362682cf54278d4c824d8ee602580336c0d155f26dffe22eb2f5257c4b`.

Every row was serialized with `flock /tmp/umber-perf-host.lock`. The
issue-private runner, interposer, binaries, census tables, and perf receipts
live under `target/umber2-7asg.11/`; they are ignored diagnostic evidence, not
production tools.

## Structural change

Synchronous integer and internal-value calls now publish into one bounded
caller-owned `ScalarCallFrame<T>`. The value and `CommandError` occupy
disjoint slots and only a one-byte complete/suspended/failed status crosses
the call boundary. The no-continuation path also writes any pending frame into
a borrowed destination instead of returning a whole scalar/error carrier.

Legacy `Result`-returning scalar entry points settle at their producing call
site. This preserves the existing typed continuation ownership: only a real
resource suspension installs the attempt's frame in the singular ABA-tagged
scratch lane, while completion consumes the value and failure consumes the
error immediately.

Architecture is simpler after the change: result delivery, terminal error,
and retained continuation have one owner each. No heap indirection, arena,
cache, warmed allocation, second continuation lane, generation lifetime, or
workload-specific path was added. The compatibility `finish_scalar_call`
adapter remains only for the structured filename boundary outside the hot
scalar call chain.

## Exact public-copy result

The preload census interposes public `memcpy` and `memmove` separately and had
zero caller-table and size-table overflow in both rows. The three audited
families started at 861,633 calls and 648,667,896 bytes. In the candidate,
`finish_scalar_call` and `scan_something_internal` have no hot large-carrier
row; `scan_integer` retains two 384-call cold 792-byte rows, totaling 768 calls
and 608,256 bytes. That is a 99.91% call reduction and 99.91% byte reduction
across the named families.

Whole-process public `memcpy` falls from 36,566,281 calls and 6,169,543,655
bytes to 35,707,144 calls and 5,521,751,636 bytes: 859,137 fewer calls and
647,792,019 fewer bytes. Public `memmove` is unchanged at 52,070 calls and
4,795,428 bytes.

The exact command-work vector is identical in every row:
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Both stop at the authenticated fuel boundary with the same
canonical diagnostic; routine tests cover completed vector and output paths.

## Cycle and allocation confirmation

Paired `cycles:u`, 199 Hz, frame-pointer captures contain 1,676 base samples
and 1,554 candidate samples with zero lost samples. Approximate weighted
cycles fall from 19,339,077,951 to 18,337,605,040, a 5.18% reduction. Copy
counts and cycle samples remain parallel evidence rather than a fabricated
per-call cycle attribution.

The focused profiling-feature scalar tests keep warmed keyword success and
failed-prefix paths at zero allocation calls and zero requested bytes. The
call frame itself is stack-owned and performs no allocation.

## Verification

`cargo test -q --tests` passes the complete routine suite. The focused
`tex-command` suite passes 242 unit and 18 integration tests, including the
one-byte status and bounded-frame structural test. `scripts/check.sh` reports
all dprint, Biome, rustfmt, and clippy gates passed.
