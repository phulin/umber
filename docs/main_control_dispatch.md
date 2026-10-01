# Main-control dispatch

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
governs this engine. `tex-command::CurrentCommand` contains its native packed
`CommandWord`; `control_code` projects its dispatch family without storing a
second decoded meaning. `main_control/direct/dispatch.rs` is the one packed
mode/command switch, including TeX82 §1030's `reswitch`, §1034's horizontal
character loop, and §1211's prefix loop. Ordinary, observed, tracked,
diagnostic, nested math/accent/alignment, PDF, and resource consumers enter
these same arms.

Expanded fetching and raw horizontal lookahead retain their distinct TeX
labels. Borrowed source spans enter the actual horizontal material consumer.
Prefixes and replacement commands dispatch in place without redelivery. Each
selected arm scans its concrete operands through the existing processor,
ends that processor/context loan, and calls its native semantic owner.
`scan_*` and `apply_*` files group concrete routines by responsibility;
`semantic/` holds source-free semantics. The large authoritative switch stays
in one file so its mode and prefix ordering is visible together.

Genuine scanner requests parameterize their token or definition fields by
ownership domain. Token publication validates every attempt root, reserves
all native rows/words, then consumes the request while streaming final token
owners. Alignment templates use the same complete preflight discipline. There
is no duplicate rooted request, operation union, phase placeholder, readiness
mailbox, eligibility lane, prepare/apply protocol, or second executor.
Definitions remain actual published keys until counted definition ownership
lands in `.13`; provisional shorthand diagnostics consume the old owner before
replacement.

`semantic_owners.rs` lends actual command, mode, box, alignment, and output
owners to a concrete call. Case-specific code publishes alignment and
assignment facts at their semantic positions. The common tail preserves show
completion, deferred print, shipout, page-output, and paragraph ordering.
Observed execution emits its ordered mutation/effect/artifact receipts;
unobserved execution constructs no receipt batch. `settlement.rs` clears real
scanner scratch, commits demanded receipts, and latches failures. It creates
no per-command savepoint or lifecycle capability.

Host misses unwind call-local scanner and semantic owners. The host restores a
full aggregate checkpoint before retry; fuel and high-water statistics remain
monotone. Genuine page, output, candidate, and aggregate owners retain their
existing cold lifetimes. Performance measurements are informative, with no
speed threshold for implementation closure.

Replay accounting counts completed driver chunks at the host boundary. A chunk
can contain several native commands; removing cold handoffs changes this
partition without changing replayed state or output. The resource-equivalence
test requires real discarded work and exact restart/response counts while
comparing complete preloaded and partitioned output byte for byte.
