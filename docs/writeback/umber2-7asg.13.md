# `umber2-7asg.13`: direct mode inverse construction

## Representation and rollback

The mode journal still owns one ordered `Vec<Inverse>`, one O(1) position per
projected field, and generation-checked nested frame cursors. The generic
`ListJournal::record_once(field, Inverse)` handoff is deleted. Each typed
operation now checks its first-write position and constructs its concrete
tagged inverse only inside that branch, immediately before the single log
insertion. The log position is published after insertion. Later mutations in
the frame neither construct nor transfer another inverse.

This is a deletion and consolidation of the default path: there is no second
log, side payload arena, whole-list snapshot, box, heap indirection,
compaction, cache, special path, new generation owner, or hidden copy. Node,
pending-horizontal-run, push, and pop inverses retain their existing ownership
and ordering. Reverse replay, nested commit/rollback, list identity, and
immediate clearing after the outermost commit are unchanged.

The focused first-write test now mutates every scalar and owned display field
twice in one frame, checks that the log remains at ten records, and verifies
the exact entry state after rollback. Existing tests continue to cover every
destructive family, nested commit and rollback, push/pop replacement, stale
cursors, and fatal commit.

## Exact copy evidence

The authenticated 20,000,000-action workload uses packed distribution root
`721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, the exact prefetch closure, and fixed source
clock. Base executable SHA-256 is
`d0dd4f0c27c7770b0c88911908c1f7545ce8a3832cb256e77c5d44cc7742199f`;
candidate SHA-256 is
`535cb7d2265745f2f793f16ba4ce4e1c813b562127295eb69eac1ea539ab4d9a`.
Both public-copy tables have zero caller and size overflow.

The baseline generic handoff comprises exactly 233,236 public `memcpy` calls
and 145,539,264 bytes at 624 bytes each:

| Inverse payload      |       Calls |           Bytes |
| -------------------- | ----------: | --------------: |
| `NoBoundary`         |     123,024 |      76,766,976 |
| `SpaceFactor`        |     106,816 |      66,653,184 |
| `PrevDepth`          |       1,334 |         832,416 |
| `PrevGraf`           |       1,142 |         712,608 |
| `IncompleteFraction` |         908 |         566,592 |
| `HyphenContext`      |           8 |           4,992 |
| `DisplayAlignment`   |           4 |           2,496 |
| **Generic handoff**  | **233,236** | **145,539,264** |

Symbolization finds no candidate `record_once` owner. One required direct
`Vec` insertion remains per first inverse, so whole-process 624-byte calls
fall from 350,272 to 233,654, a reduction of 116,618 calls and 72,769,632
bytes. The larger benefit comes from applying the first-write guard before
variant construction: repeated same-frame writes no longer create a value to
discard. Whole-process public `memcpy` falls from 35,364,432 calls and
5,740,579,119 bytes to 33,499,645 calls and 4,912,446,134 bytes: 1,864,787
fewer calls (5.27%) and 828,132,985 fewer bytes (14.43%). `memmove` changes
from 51,947 calls and 4,768,860 bytes to 52,070 calls and 4,795,428 bytes.

## Identity, cycles, and controls

Every warmed census and frame-pointer row intentionally returns status 1 at
the exact fuel boundary and reproduces
`(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Standard output is byte-identical and empty; neither row
publishes an output artifact.

The accepted base/candidate census and perf sequence held
`flock /tmp/umber-perf-host.lock` throughout. Saved process censuses contain
no Cargo or rustc peer, and CPU full-pressure `avg10` remains zero at every
boundary. The base capture has 1,597 samples, zero lost samples, and
18,952,647,032 weighted `cycles:u`; the candidate has 1,469 samples, zero lost
samples, and 17,342,226,487 weighted cycles, 1,610,420,545 fewer (8.50%). The
cycle change is supporting host evidence; the exact copy census and identity
vector are the representation authorities. Issue-private binaries, tables,
profiles, and process receipts remain under `target/umber2-7asg.13/`.

## Verification

The focused journal tests, complete `tex-exec` tests, full routine
`cargo test -q --tests`, and `scripts/check.sh` pass. The last command is the
authority for dprint, Biome, rustfmt, and clippy.
