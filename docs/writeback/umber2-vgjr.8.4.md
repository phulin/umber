# umber2-vgjr.8.4 — state façade API disposition

Authority: [`Universe`](../../crates/tex-state/src/universe.rs), based on source
snapshot `070dc6408`.

The public-consumer audit found exactly two implementations of the exported
`ExpansionState` trait (`Universe` and `ExpansionContext`) and no production
generic consumer in `tex-command`, `tex-exec`, or any other workspace crate.
The remaining generic signatures were token-rendering helpers owned by
`tex-state`; the only downstream import was stale test scope. `Stores` was
already unreachable downstream because `stores` is a private module and the
type is not re-exported.

The API disposition is therefore internal-only, with no deprecation adapter.
This pre-1.0 workspace has no demonstrated external implementation to preserve.
`ExpansionState`, `ExpansionContext`, `MeaningCacheGuard`, and the two large
forwarding implementation bands are removed. Token rendering takes `Universe`
directly, making it the sole state-facing API. `CommandContext` remains the
borrow-scoped command capability and `InputOpenContext` remains the distinct
host-input capability. The private `Stores` aggregate is retained strictly as
implementation data coordinating atomic state; it is not a public or retained
compatibility façade.

The retired cache guard had no consumer once the forwarding trait disappeared,
so its generation field and profiling-only invalidation counters were deleted.
Owner address-and-nonce checks, snapshot rollback, survivor and timeline pins,
group-invalidated snapshots, dependency observation, handle liveness, and
exclusive transaction borrows remain on their existing aggregate paths. The
compile-fail input and arena boundaries remain; obsolete tests for the removed
expansion wrapper were deleted instead of preserving a false capability.

Before this writeback, the implementation diff measured 61 additions and 1,643
deletions in production Rust, a net production deletion of 1,582 lines. Tests
measured 1 addition and 52 deletions, a net test deletion of 51 lines. The
net reduction includes 1,449 lines from `universe.rs` plus the dead guard,
measurement, forwarding, and fixture residue in adjacent files.

Validation was serialized as required:

- `tex-state` compiled uncapped with `--no-run`, then 728 unit tests and the
  external-boundary suite passed under a 512 MiB cgroup;
- `tex-command` and `tex-exec` compiled together uncapped with `--no-run`, then
  their unit and integration suites passed under a 512 MiB cgroup;
- the full workspace compiled uncapped with `--no-run`, then passed under a 1
  GiB cgroup; and
- `scripts/check.sh` passed all four gates under the 1 GiB cgroup. Cargo build
  concurrency was limited to one after the first parallel clippy attempt
  exceeded that cap; both lint passes were clean across 28 workspace members.
