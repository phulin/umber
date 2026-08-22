# Phase 8 Runtime Performance Acceptance

This document records the final-runtime architecture audit and the reproducible
performance authority for `umber2-66p0.8`. It measures the branded HRTB runtime
that phase 7 left in place. Deleted live-handle, ownership-registry, snapshot,
clone, fork, rehome, and compaction designs are not benchmark compatibility
surfaces.

## Static hot-path contract

The audited ordinary paths are command delivery and expansion, meaning lookup,
macro argument scratch, scalar and code-table scanning, assignment, page-list
construction, operation rollback, retained-session resume, and incremental
accept/reject. The following properties are required together:

- `CommandContext::meaning` and dense register reads admit once and resolve by
  direct packed slot or bank index. Runtime ids do not perform generation or
  owner registry searches.
- Durable definitions, token lists, glue, nodes, and mutable font state resolve
  by packed/direct arena coordinates. Their values carry no per-value `Arc`,
  `Weak`, owner, registry entry, or forwarding pointer.
- Attempt token buffers retain their backing capacity in the operation arena's
  recycled scratch pool. Rejection truncates coordinates and returns buffers
  to that pool instead of cloning the token graph.
- Page insertion-class reads use a dense class-to-position index. The one-time
  activation edge keeps the separate iteration vector in canonical class order;
  ordinary reads and updates perform no binary search.
- Incremental history contains detached boundary and output evidence only. A
  session has one accepted prior owner and at most one candidate current owner.
  Rejection drops current; acceptance drops prior and promotes current. No
  compactor, relocation graph, slab splice, or additional history owner exists.
- Engine crates forbid unsafe code. The only runtime-adjacent unsafe block is
  the isolated profiling allocator's exact forwarding implementation, which is
  absent unless the profiling feature is selected.

The two remaining `Weak` uses are coarse boundaries, not value ownership:
`Universe` may borrow the optional session-level pure-memo capability, and the
retained-generation lifecycle test API exposes a weak witness to one coarse
owner. Neither is consulted by an ordinary value read or mutation.

## Profiling boundary

`tex-state/profiling` compiles a process-local allocator and structural census.
The feature attributes only named hot phases, attempt scratch, coarse generation
construction, arena growth, and explicit cold materialization. It also counts
coarse retained-generation creation, drop, peak live owners, and explicit
terminal retirement. Non-profiling builds contain no counter fields, branches,
or atomics.

The final state gate warms its storage before measuring and requires zero
allocation calls and bytes for one million direct meaning/register reads,
16,384 assignment/rollback cycles, and a 16,384-node page-queue round trip. It
also requires exactly two simultaneous coarse generation owners, exact current
drop on rejection, exact prior drop on acceptance, and zero owners after the
terminal retirement.

## Authoritative fixed workload

The numerical authority is the pinned pdfLaTeX build of arXiv `2606.12566`
under the immutable TeX Live 2026 distribution and resource closure. The host
environment is `SOURCE_DATE_EPOCH=1787080434` and `LC_ALL=C.UTF-8`. Inputs are
accepted only at these identities:

- source SHA-256:
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- format SHA-256:
  `32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`;
- distribution SHA-256:
  `560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`;
- 105-key closure SHA-256:
  `75d85bb12f8fa5eba0ae2a42daf73fd86c44852ecdc230196455b9aea24565b5`.

The accepted output must preserve the declared semantic coordinates and output
identity recorded by the pinned authority. The fixed budgets are no more than
20 seconds wall time and 150 MiB peak RSS. Measurements are authoritative only
in a quiet window after repository provisioning verifies the locked assets;
default hosted resources are not a substitute.

## Reproducible gates

The focused commands are:

```bash
scripts/check-snapshot-budgets.sh
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin packed_cutover_gate
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations -- --perturb
cargo bench --manifest-path benchmarks/tex-incr/Cargo.toml \
  --bench accepted_edit
```

The state script retains its established operator entry point, but now runs the
final state/generation gate rather than the deleted snapshot-retention model.
