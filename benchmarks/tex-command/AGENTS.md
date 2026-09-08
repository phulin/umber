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
