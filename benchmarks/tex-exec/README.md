# tex-exec benchmarks

This standalone crate contains focused execution-layer benchmarks that are
excluded from the root workspace correctness gate.

Run the shipout lowering cases with:

```bash
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench shipout
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench widths
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench layout
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench dvi_page_snapshot
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench mode_list_rollback
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench pure_memo_edit
cargo run --release --manifest-path benchmarks/tex-exec/Cargo.toml --bin layout_allocations
```

`ordinary_hlist` measures the normal artifact-lowering fast path.
`deferred_math_lists` measures shipout-local Appendix G conversion for frozen
math lists that survived into a shipped tree. Both cases lower 1,024 child
nodes. Each Criterion iteration builds fresh state outside the timed region,
then times execution and artifact commit.

`dvi_page_snapshot` compares the former deep clone of one bounded synthetic
1 MiB DVI page plan with the shared immutable collection clone used by
canonical aggregate-operation rollback. It does not use a document trace.

`mode_list_rollback` isolates 1,024 successful appends to a synthetic
16,384-node list. It compares the former retained-COW-root lifetime with the
length watermark used by the adopted append-aware inverse journal. The
benchmark records the completed design choice; it is not a correctness model
for destructive list edits or nested savepoints.

`pure_memo_edit` compares an accepted edit with the bounded pretolerance,
page-breaking, and shipout memo runtime disabled and enabled. It uses a real
incremental session and verifies the initial cold run before timing the edit.
It measures the retained pure caches, not the deleted paragraph replay design.

`widths` measures exact hpack width accumulation for 64- and 4,096-character
same-font runs and a 4,096-node mixed-font/interrupted list. It uses fixed
synthetic immutable TFM metrics, prepares arena state outside the timed loop,
and is the kernel budget for compact node-word width scans. The committed
means were remeasured after generation-tagged `NodeListId` expanded to two
words; the gate permits 10% cross-run noise above them. Absolute timing is
machine-specific, so comparisons require the same host, toolchain, profile,
and rebuilt revision.

`layout` covers a 4,096-cell alignment with adversarial spans, a 1,024-node
paragraph, 20,000 nested math choices, 20,000 structural sub-mlists (both also
act as stack-safety gates), and repeated 1,024-noad conversion. The allocation
gate retains the larger 4,096-node paragraph workload. `layout_allocations`
measures the same pure kernels outside workload setup and enforces committed
ceilings for allocation count and total allocated bytes; it remains outside
the ordinary unit-test tier.
