# `umber2-66p0.34`: singular processor fuel ownership

## Evidence boundary

The paired comparison uses the authenticated arXiv `2606.12566` workload,
packed distribution root `721e833071d92bba`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0`, the exact 123-key closure, a fixed source clock,
and the 20,000,000-action fuel boundary. The baseline frame-pointer profiling
executable has SHA-256
`cea3a1867bfc8441a191111b52f4a791b85e902b64759a7ba7dc4e3d6d6be00f`;
the candidate has SHA-256
`4d3d3e5cb3bfa7d96e7c202620153159be703b01c9829688757e3c88fb9e7a02`.

The accepted control and perf rows were serialized with
`flock /tmp/umber-perf-host.lock`. Both zero-loss perf rows began and ended at
CPU-pressure `avg10=0.00`; process receipts show no running Cargo, rustc,
Umber, or perf peer. Issue-private binaries, perf data, reports, process and
pressure receipts, and build logs live under `target/umber2-66p0.34/`.

## Ownership proof and structural change

The semantic fuel owner remains `MainControl::fuel`, a single
`CommandFuelLedger` containing the limit and all six monotonic counters. In
the baseline, `CommandProcessor` instead stored `ProcessorFuel`, whose
`Owned(CommandFuelLedger)` and `Shared(&mut CommandFuel)` variants forced
every charge through an ownership branch before reaching the one
`CommandFuel::charge` implementation. The owned variant was redundant:
standalone processors need the same ledger semantics as executor processors,
and their caller can own that ledger for exactly the processor lifetime.

The candidate deletes `ProcessorFuel`, the owned constructor, and the
`with_fuel` constructor split. Every processor now receives one ordinary
`&mut CommandFuel` from its caller and charges it directly. Executor sessions
still lend `MainControl::fuel`; standalone tests and the command-stream tool
create one call-local ledger and lend it across the same delivery or replay
episode. `CommandFuel::charge`, `charge_many`, exhaustion construction, and
the six counters are unchanged, so no work can be refunded by rollback,
retry, or suspension.

## Architecture simplicity

The default path now has one fuel representation, one processor constructor,
one ledger owner, and one charge implementation. The change deletes an enum,
one duplicate constructor, a mutating constructor handoff, and the branch on
every charge. It adds no cache, fast path, heap indirection, generation owner,
counter copy, special processor, compaction, or lifetime registry. Macro
scratch and recovery transitions are unchanged.

## Exact semantics and cycle result

All four rows reach the identical command-work vector
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Baseline and candidate stop at the same fuel boundary with
status 1, identical canonical diagnostics, empty standard output, and no
published output artifact. The two control diagnostics have identical
SHA-256 `68031179ff7c37a0902ed1181ea753addeb0ea80ebc5f38881ed24fb40ac85b1`.

The baseline capture contains 1,508 samples and 17,874,257,621 approximate
weighted cycles; the candidate contains 1,509 samples and 17,978,903,035
cycles. Total sampled cycles rise 104,645,414 (0.59%), so the whole-program
row is supporting context and is not labeled a benefit. At the changed
boundary, the baseline `ProcessorFuel::charge` accounts for 44 self samples,
529,173,807 self cycles, and 626,575,061 ancestry cycles. The candidate's
single `CommandFuel::charge` accounts for 28 self samples, 344,986,363 self
cycles, and 418,962,971 ancestry cycles: reductions of 184,187,444 self
cycles (34.8%) and 207,612,090 ancestry cycles (33.1%). `ProcessorFuel` is
absent from the candidate symbol table. Both captures report zero lost
samples.

## Verification

Focused `tex-command` and `tex-exec` suites pass 244 and 695 unit tests plus
their 18, 4, and 23 integration/fixture groups. `cargo test -q --tests` passes
the complete routine workspace suite. The architecture regression forbids
`ProcessorFuel` and `with_fuel`, and pins one processor constructor.
`scripts/check.sh` passes dprint, Biome, rustfmt, and both clippy resolutions.
