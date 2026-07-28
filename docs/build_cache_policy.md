# Build Cache Policy

Status: standing repository policy

Cargo build outputs are checkout-local by default. Linked Git worktrees each
resolve `target/` beneath their own root; they share neither ordinary test
artifacts nor `target/clippy` unless a caller explicitly sets
`CARGO_TARGET_DIR`.

The standing decision is to preserve Cargo incremental compilation for normal
human and agent development. The repository does not set
`CARGO_INCREMENTAL=0` globally. Clippy remains isolated in `target/clippy` so
it does not contend for the routine test target's Cargo lock.

## Measured Budget

The 2026-07-28 audit measured:

- 321 GiB free on `/home` at audit time, compared with the historical 13 GiB
  incident that motivated `umber2-johp.152`;
- 25 GiB in the long-lived primary target, including 12 GiB in
  `target/debug/incremental` and 3.4 GiB in `target/clippy`;
- 6.8 GiB in a representative built worktree, including 3.0 GiB incremental
  and 1.4 GiB clippy state;
- a historical 9.6 GiB single-worktree peak recorded by the issue.

Capacity policy therefore rounds the observed peak up to 12 GiB per newly
dispatched worktree and retains another 4 GiB as a filesystem reserve. A
one-job dispatch requires 16 GiB available; two jobs require 28 GiB. This is a
dispatch floor, not an assertion that every build consumes the full budget.

Run the deterministic preflight before starting a wave:

```bash
scripts/build-cache-policy.py --jobs N
```

The default is report-only. A refusal means the coordinator must reduce the
wave, reclaim an idle checkout explicitly, or move outputs to storage whose
capacity has separately been checked.

## Reclamation

Reclamation is opt-in:

```bash
scripts/build-cache-policy.py --reclaim --jobs N
```

It acts only on the checkout resolved by Git and only on these exact,
regenerable directories:

- `target/debug/incremental`
- `target/clippy`

The tool refuses symlinked targets, paths escaping the checkout, non-directory
targets, and reclamation while `cargo`, `rustc`, `rustdoc`, or `clippy-driver`
is active from that checkout. It never scans for broad deletion candidates and
never automatically reclaims merely because capacity is low. Remove abandoned
worktrees after integration rather than accumulating their independent targets.
