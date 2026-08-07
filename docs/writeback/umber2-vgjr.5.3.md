# umber2-vgjr.5.3 — canonical artifact-derived DVI

Issue: `umber2-vgjr.5.3`

Implementation tree: `dcb9fbda7f920cc7b8909f5f10e29f78e7a134d5`

TeX82 §§638--642 define shipout and list-output behavior. Umber now lowers
live page state once into canonical artifact bytes, then derives fresh and
memo-hit DVI through the same `DviPagePlan::compile_v10` authority and the
same executor error conversion. DVI-disabled fresh and memo publications both
retain artifacts while omitting page plans.

The retired predecessor was the paired live artifact/DVI emission path in
`tex-exec`, including DVI parameters threaded through every node emitter,
streamed-plan selection, and the complete 336-line live-node leader
materializer. No fallback or shadow comparison path remains. Positioned PDF
effects still lower the committed artifact for saved-position and snapping
state, preserving their existing state publication boundary.

Production Rust changes by +34/-538, a 504-line net reduction. Active Rust
proof changes by +79/-1; guidance changes by +2/-2. Before this writeback and
the combined-plan update, the complete tracked issue changes by +115/-541, a
426-line net reduction. No fixture or serialized artifact changed.

Verification on the implementation tree:

- `tex-exec`: 20 passed under `MemoryMax=512M`; `tex-out`: 145 passed under the
  same cap.
- Exhaustive `tex-command-stream`: `VERDICT: CLEAN`, zero semantic and zero
  advisory geometry divergences.
- Byte-exact Story and canonical Gentle DVI corpus gates passed.
- The 1,024-node shipout stop gate improved ordinary throughput about 12% and
  deferred-math throughput about 42% against its stored predecessor baseline;
  maximum RSS was 49,836 KiB under `MemoryMax=512M`.
- `cargo test -q --tests`: complete native suite passed under `MemoryMax=1G`,
  maximum RSS 315,388 KiB.
