# Command benchmark guidance

This directory contains synthetic, self-contained command-core benchmarks.
Keep the existing profiling allocation and checkpoint gates stable. The
`timing/` package is the feature-free release timing tier; it must not enable
the `profiling` feature or install a custom allocator. The profiling wrapper
uses the same matrix source only for optional structural receipts.

Matrix fixtures must keep setup and warmups outside the timed region, use
fresh equivalent source and stored inputs, validate semantic output, and print
stable JSON Lines records. Do not add engine production changes for benchmark
instrumentation.

The consumer runner is split by responsibility: `src/command_consumer_core.rs`
owns the CLI, shared consumer state, and workload dispatch;
`src/command_consumer_text_definition.rs` owns long text and definition
scanning; `src/command_consumer_chain_mixed.rs` owns macro-chain and mixed
pipeline loops; `src/command_consumer_support.rs` owns processor setup,
counter accounting, and receipt validation; and
`src/command_consumer_fixtures.rs` plus
`src/command_consumer_fixture_body.rs` provide synthetic inputs and body
expectations, while `src/command_consumer_receipts.rs` formats JSONL output.
The feature-free timing entry point is `timing/src/consumer_main.rs`; the
profiling wrapper is `src/bin/command_consumer_profile.rs`.
The reproducible paired release workload is
`benchmarks/tex-command/consumer-manifest.json`.
