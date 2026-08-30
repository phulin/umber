# tex-command benchmarks

This standalone crate contains focused command-core benchmarks and is excluded
from the root workspace correctness gate.

Run the allocation-count baseline with:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations
```

`command_allocations` directly exercises single-token backup, macro argument
matching, `scan_toks` absorption, keyword and dimension scanning, alignment
preamble scanning, two-token `off_save` recovery, rendered-token installation,
command-text rendering, token-list iteration, case shifting, macro-definition
collection, `\read` token collection, output replay expansion, and
control-sequence tokenization for both inline and pathological spill names. The
recovery and rendered-token rows detect fixed-array-to-`Arc` staging regressions
separately from the common single-token backup row. Each row reports allocation
count and requested bytes per operation after three discarded operations warm
the command core's bounded process-local scratch pool. The program builds fixed
cases before measurement and executes 64 independently measured operations. The
reported value is the final representative operation, after allocator and
scratch state have settled, using the same `stats_alloc::Region` convention as the
`tex-state` and `tex-exec` allocation gates.

The `unobserved` configuration has no external observer.
`external_observer` attaches a non-allocating sink so the counts include
observation payload construction. Pure command-text rendering supports only
`unobserved`, because it has no processor observation boundary. That workload
clears and reuses a caller-owned render buffer around the public append API, so
it measures renderer-internal allocation rather than the ownership allocation
deliberately retained by the convenience wrapper.

To verify sensitivity, add `--perturb`. It deliberately requests one 64-byte
allocation per measured operation, so every reported row increases by exactly
one allocation and 64 requested bytes. The values are diagnostic baselines,
not correctness-test ceilings; optimization issues should record before and
after output from the same host, toolchain, profile, and revision.

The owned tokenizer-name inline bound is 24 semantic characters. A repository
fixture census found 9,770 control-word occurrences with median 5, p95 15, p99
20, and maximum 31 characters; all 199 registered primitive-name literals
were at most 17 characters. The bound therefore covers more than 99% of the
measured source workload and every primitive while keeping the benchmark's
long-name row on the required unbounded spill path. Inline raw delivery also
encodes character codes into a fixed stack UTF-8 buffer before lookup or
interning; only an already-spilled pathological name constructs a temporary
`String`.

`packed_cutover_gate` additionally times one million warmed single-token
backup/replay cycles; known creating, known non-creating, and stored-token
control-sequence deliveries; 65,536 genuinely new creating and unknown
non-creating source names; one million name-based and packed immutable
primitive resolutions; 16,384 warmed failed-keyword scans; five million
uniform stored-cursor calls split evenly across replay, macro replacement,
macro argument, attempt, and durable owners; and five million indexed reads
across one sealed 16,385-word macro argument. These rows separate source decode,
lookup/probe, TeX-visible creation, packed meaning delivery, backed-up raw
delivery, mixed packed-cursor traversal, and long segmented argument access.
The destination-directed row warms and then measures 8,192 calls apiece to raw
non-creating, raw creating, and expanded delivery, reusing one caller-owned
command slot throughout. All three policies must remain allocation-free.
The fused raw/expanded row then measures one million ordinary stored-character
deliveries through each policy. It requires exactly two million resident-frame
steps, one million expanded completions, no meaning lookup, and zero warmed
allocation; its separate raw and expanded timings are sized for direct
`cycles:u`/`instructions:u` and public `memcpy`/`memmove` comparison.
The macro-matching row also asserts that its successful first-token expansion
does not increase the matched-word read counter: paragraph and removable-
outer-group decisions must consume first-scan metadata rather than reread the
staging span.
The mixed row reports absolute
calls, exact end-of-span retirements, one exact nonzero scalar rollback,
elapsed time, and a semantic checksum. Stored replay loops assert zero
allocation calls and requested bytes after their input storage has reached high
water. Pass `--only=<row>` to run one named row in isolation under a
hardware-counter tool such as `perf stat`; every timed row prints its operation
count so the absolute counter can be normalized. The legacy
`--mixed-stored-only` spelling remains an alias for the mixed stored-cursor
row. Direct-source delivery retains its real
append-only provenance rows, so its timing loop relies on the separate
`command_allocations` rows for allocation comparison. Every timing row reports
nanoseconds per complete operation for local before/after comparisons. The
time is diagnostic rather than a correctness ceiling; the allocation
assertions and structural size gates are deterministic.

Run the reversible command-timeline promotion gate with:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_checkpoint_gate
```

`command_checkpoint_gate` compares one live command unit with 64 accumulated
source and stored-token units. It enforces identical zero allocation calls and
requested bytes for warmed capture, checkpoint clone, same-generation restore,
the first scalar mutation after capture, command-only candidate fork, the
first mutation after fork, and one warmed obsolete-frame release. It
additionally performs 8,192 writes to one hot
scalar in a single interval and requires one packed record, 8,191 coalesced
writes, zero scalar-list descriptor publications, at most 32 record bytes, and
zero warmed heap allocation. A second 8,192-transition loop repeatedly pushes
and pops one input frame after warming its physical depth; it requires zero
allocation, no new logical undo record, and no additional displaced payload.
The source-history fixture separately requires zero allocation for 8,192
copy-small lexer mutations (one stored-state capture and 8,191 coalesces) and
for one cold loaded-line owner swap. After warming, it also measures compact
and cold-owner source inverses followed by pop, physical-row token replacement,
and replacement-frame mutation. Both ordered-reuse rows require zero allocation,
zero full source/frame clones, and exactly the required inverse plus replacement
records; the cold row must add exactly one ordered owner swap.
An independent source-depth row compares one and 4,096 live source slots. In
both cases, 4,096 warmed lexical mutations must allocate zero bytes, append one
inverse of at most 48 bytes, coalesce the other 4,095 mutations, perform no
source-owner swap, and clone no whole input frame.
The gate also rejects a candidate mutation, releases one obsolete prefix, and
forks the surviving accepted mark again to prove exact rollback, lineage
isolation, and prefix-floor validity. Finally it measures 10,000,000 successive
mutation/boundary/release cycles and requires one live frame, one 128-row frame
page, and one physically returned journal chunk per boundary. Elapsed time is
reported diagnostically; physical occupancy is the deterministic gate.
