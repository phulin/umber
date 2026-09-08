# tex-command benchmarks

This standalone crate contains focused command-core benchmarks and is excluded
from the root workspace correctness gate.

## Consumer measurement runner

The consumer runner measures eight synthetic rows: long text, macro-definition
scanning, a parameterized macro chain, and a mixed text/macro/definition
pipeline, each with source and stored-token input. It installs all catcodes and
macro definitions in a fresh in-memory universe, so no TeX Live assets are
needed. Both binaries use the same fixture and workload code:

```bash
cargo run --release --manifest-path benchmarks/tex-command/timing/Cargo.toml \
  --bin command_consumer_timing -- --workload all --storage both \
  --iterations 512 --warmups 2 --text-chars 4096 --body-words 8193
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_consumer_profile -- --workload all --storage both \
  --iterations 512 --warmups 2 --text-chars 4096 --body-words 8193
```

The timing package has no profiling feature or custom allocator and is the
wall-time tier. The profiling wrapper appends structural receipts and its
elapsed values are diagnostic. `--workload` accepts `all`, `long_text`,
`definition_body`, `parameterized_chain`, or `mixed_pipeline`; `--storage`
accepts `source`, `stored`, or `both`. `--text-chars` and `--body-words` are
honored by every workload that uses them and are printed in the metadata line.

Setup, semantic preflight, warmups, and end-of-input validation are outside the
timer. The measured loop reuses the warmed command cursor and semantic state;
the short-lived processor borrow is renewed only to take an actual fuel
snapshot before timing. Rows mark the timed checksum consumer explicitly with
`"timed_evidence":"checksum_sink"`, so the reported elapsed time includes
the checksum sink as part of consumer cost. Final definitions, independent
fixture probes, semantic hashes, fuel, and profiling counters are checked
after timing. Use `scripts/paired-measure.py` with the release binary for
before/after measurements. The tracked
`benchmarks/tex-command/consumer-manifest.json` gives each workload its own
loop count, including a 100,000-operation chain and a 10,000-operation mixed
row; compare the profiling lane separately when structural counters are
needed.

Run the allocation-count baseline with:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations
```

For command-core wall time, use the feature-free timing package:

```bash
cargo run --release --manifest-path benchmarks/tex-command/timing/Cargo.toml \
  -- --case=all --iterations=100000 --warmups=64 --observer=both
```

This is the release baseline. Its dependencies do not enable `profiling`, and
it installs no custom allocator, counters, or allocation interposer. The
optional profiling receipt uses the same fixtures and arguments:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_core_profile -- --case=all --iterations=100000 --warmups=64 \
  --observer=both
```

Do not compare the profiling wrapper's elapsed time with the release tier. It
enables the existing profiling feature and `HotCoreAllocator` only to append
structural receipts such as macro-expansion counts and raw-delivery lanes.
Use the release command for before/after timing.

The matrix emits one metadata JSON line followed by one JSON line for every
case, storage, delivery, and observer combination. `--case` accepts `all` or
one of `plain`, `empty_macro`, `nonempty_macro`, `parameterized_empty`,
`parameterized_identity`, `balanced_argument`, and
`delimited_nested_argument`. `--observer` accepts `disabled`, `enabled`, or
`both`; `--iterations` is the timed count and `--warmups` may be zero. The
default matrix has 7 cases × 2 storage forms × 3 delivery policies × 2
observer modes = 84 records.

The fixture meanings are deliberately small and fixed. `plain` delivers one
letter. `empty_macro` calls a no-parameter macro whose empty replacement is
followed by `x`. `nonempty_macro` calls a no-parameter macro replacing itself
with `z`. `parameterized_empty` consumes one braced `a` argument and then
delivers `x`; `parameterized_identity` consumes one unbraced `a` and replays
it. `balanced_argument` consumes `{a{b}}`; `delimited_nested_argument`
consumes `{a{b}}|` before replacing the result with `z`. Braces are assigned
their TeX begin/end-group catcodes in the synthetic universe. Raw cases use a
one-control-sequence input per operation, while expanded cases use the full
macro call; source and stored forms therefore have fresh, equivalent semantic
streams for the selected delivery policy.

Each record reports elapsed nanoseconds and nanoseconds per iteration, an
independently expected semantic checksum and hash, and the known source-word,
stored-word, selected-input-token, and replacement-body denominators. The
`token_work` denominator is selected input tokens plus replacement-body tokens;
`macro_calls` is reported separately. Argument and delimiter tokens remain
separate fields so a change that preserves output while changing scanner work
cannot pass by normalizing everything into one count. Totals are measured-count
normalizations of the per-operation fields. Setup, the semantic preflight,
warmups, and the end-of-input sentinel are outside the timed interval. The
preflight checks the delivered token and control-sequence identity on a fresh
input, and the sentinel rejects an unexpected suffix. The timed loop only
accumulates cheap checksum and token-shape evidence, which is checked against
the independent fixture expectation after timing.

The existing command benchmarks remain available with these exact commands:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations [-- --perturb]
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin packed_cutover_gate [-- --only=<row>]
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_checkpoint_gate
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin resident_macro_body_gate
```

`command_allocations` covers `single_token_backup`,
`macro_argument_matching`, `scan_toks_absorption`, `keyword_scanning`,
`dimension_scanning`, `alignment_preamble_scanning`,
`two_token_off_save_recovery`, `rendered_token_installation`,
`command_text_rendering`, `token_list_iteration`, `shift_case`,
`macro_definition`, `read_token_collection`, `output_replay_expansion`,
`inline_control_sequence_tokenization`, and
`spilled_control_sequence_tokenization`, each in `unobserved` and
`external_observer` configurations where supported. `--perturb` is the
existing deliberate allocation sensitivity check.

`packed_cutover_gate --only=<row>` selects one of
`ordinary_source_delivery`, `packed_backup_and_replay`,
`warmed_backup_push_pop`, `stored_token_replay`,
`warmed_mixed_stored_cursor`, `warmed_long_macro_argument_cursor`,
`known_name_lookup`, `primitive_resolution`,
`source_known_creating_delivery`, `source_known_probe_delivery`,
`source_new_creating_delivery`, `source_unknown_probe_delivery`,
`stored_control_sequence_delivery`, `direct_command_delivery`,
`macro_argument_matching`, `macro_argument_append`,
`warmed_keyword_mismatch`, `destination_directed_warm_delivery`,
`fused_raw_expanded_delivery`, `destination_owned_macro_expansion`,
`mixed_macro_resident_pipeline`, `stationary_scan_toks_progress`, and
`direct_definition_scanning`. These rows measure source decoding and lookup,
packed and backed-up delivery, macro argument and body traversal, scanner
progress, definition publication, and the direct command carrier. The
`--mixed-stored-only` spelling remains an alias for its documented mixed
stored-cursor row.

`command_checkpoint_gate` measures warmed capture, clone, restore, fork,
rollback, coalesced scalar and input-frame mutations, source-history reuse,
cursor structure, and journal-prefix release. `resident_macro_body_gate`
measures resident macro-body reads at one, 4,096, and 8,193 words, including
chunk-boundary transitions and owner-retention receipts. These gates retain
their existing deterministic allocation and structural checks; their elapsed
times are diagnostic.

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
macro argument, attempt, and durable owners; and five million absolute reads
across one sealed 16,385-word macro argument. These rows separate source decode,
lookup/probe, TeX-visible creation, packed meaning delivery, backed-up raw
delivery, mixed packed-cursor traversal, and long segmented argument access.
The direct-command-delivery row runs one and 4,096 rounds across nine table-backed
undefined, primitive, macro/alias, register, parameter, font, static-alias, and
active-character meanings. It requires one dense probe and one meaning-tag
decode directly into the caller-owned command slot per delivered command, one
final owner acquisition per macro row, zero duplicate owner acquisitions, zero
whole-meaning or command copies, and zero warmed heap allocation.
The destination-directed row warms and then measures 8,192 calls apiece to raw
non-creating, raw creating, and expanded delivery, reusing one caller-owned
command slot throughout. All three policies must remain allocation-free.
The fused raw/expanded row then measures one million stored control-sequence
deliveries through each policy across exact known replay, attempt-local, and
durable spans. The fixture shape proves the admitted domain volumes without a
per-token profiling counter in the production transition. It requires zero
intermediate stored-advance relays, two million fuel charges and resident-frame
steps, two million meaning lookups, one million expanded completions, and zero
warmed allocation; its separate raw and expanded timings are sized for direct
`cycles:u`/`instructions:u` and public `memcpy`/`memmove` comparison.
The destination-owned macro-expansion row drives one million empty macro calls
through one expanded request and its one reusable command destination. It
requires one expansion per macro, one final expanded delivery, and zero warmed
allocation, and reports time per expansion for before/after hardware-counter
comparison of the expansion result carrier.
The mixed macro/resident row then expands one million empty macros before one
parameterized macro replays its one-token argument one million times. Its
exact two million macro-body transitions cover both depleted-body retirement
and parameter words, while the same receipt reports parameter pushes, replay,
raw and expanded deliveries, macro expansions, suspension moves, command
copies, allocation, and elapsed time. The fixture derives those known domain
volumes at its boundary: ordinary profiling code carries no per-word resident-
body census. Run it under the profiling-only public-
copy interposer in `scripts/copy-attribution/` to append exact `memcpy` and
`memmove` calls and bytes from that same focused process; unlike the broad
corpus profiles, this gate intentionally stops at the two-million-transition
pipeline.
The stationary `scan_toks` row performs one million complete warmed balanced-
text scans and commits each attempt before starting the next. It requires zero
warmed allocation and prints the exact scan count for normalized
`cycles:u`/`instructions:u` and public `memcpy`/`memmove` comparison of the
scanner phase/progress carriers.
The direct-definition row scans 500,000 warmed global definitions through one
resident source and open attempt. It pre-reserves the definition and provenance
arenas, requires zero measured allocation, and reports exact semantic-word,
header, post-publication origin-write, and second-token-traversal counts. This
separates collector self work and final-region publication from resident
command-delivery ancestry without changing production allocation behavior.
The macro-matching row also asserts that its successful first-token expansion
does not increase the matched-word read counter: paragraph and removable-
outer-group decisions must consume first-scan metadata rather than reread the
staging span.
The macro-argument-append row warms and then scans one 1,000,002-token braced
argument through the canonical matcher. It requires zero warmed allocation and
reports the exact accepted-token and raw-delivery volumes for focused
instruction and public-copy comparison of the execution-scratch append
transition.
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
The coalesced counts are derived at this measurement boundary from the known
mutation attempts minus the observed first-touch records. Production command
state stores no coalesced-mutation census and performs no counter update after
a row or scalar is already touched.
The cursor-structure row independently varies group-stack depth from one to
4,096 and aftergroup payload from one to 65,536 words. It requires identical
fixed summary/cursor sizes, a one-word cursor, zero warmed capture/restore
allocation, zero logical history records, and zero full-payload clones. This
proves the checkpoint envelope neither duplicates private stack descriptions
nor traverses the aftergroup payload lane.
The gate also rejects a candidate mutation, releases one obsolete prefix, and
forks the surviving accepted mark again to prove exact rollback, lineage
isolation, and prefix-floor validity. Finally it measures 10,000,000 successive
mutation/boundary/release cycles and requires one live frame, one 128-row frame
page, and one physically returned journal chunk per boundary. Elapsed time is
reported diagnostically; physical occupancy is the deterministic gate.
