# umber2-vgjr.8 — executable state and node schema closeout

Audited implementation tree: `1c71d4afb8f6aa34e1f506a5100dc8a597ed8463`.
All four children are closed and integrated.

## Surviving authorities

`tex-state::node_arena::schema` owns the exhaustive 24-kind logical node
grammar. Its allocation-free `NodeRef` views, descriptors, semantic and
diagnostic field policy, typed content and child handles, and ordered child
events are the common authority used by equality, live-reference validation,
semantic identity, format capture, graph traversal, and survivor-remap
validation. The exhaustive owned/compact equivalence test fixes tags, fields,
handle policy, origin exclusion, and child order.

The compact `NodeWord` and structure-of-arrays sidecars remain private storage
specializations. Their tag dispatch encodes, decodes, copies, and patches
physical rows without defining a second semantic model. The handle-free
`FormatNode` DTO remains the separately validated portable schema-11 codec;
format capture consumes borrowed `NodeRef` values and canonical schema child
order. Survivor copying remains iterative and memoizes a span before patching
children, preserving shared and overlapping spans without recursion.

`Universe::from_format` validates the schema-11 container and delegates once
to `Stores::decode_frozen_format`, which decodes and cross-validates the frozen
core, non-node, node, and environment sections before publishing immutable
bases. This is the only production format restoration path. The former
test-only `StoreFormat` replay, raw store restoration helpers, replay
instrumentation, and alternate recursive aggregate/node hashing are deleted.
Memo node and font import retain their distinct bounded detached-envelope
contract and are not format restoration.

## API disposition and retained invariants

The public-consumer audit found no production generic consumer or demonstrated
external implementation of the former `ExpansionState` trait. Under the
pre-1.0 workspace-internal policy, `ExpansionState`, `ExpansionContext`,
`MeaningCacheGuard`, and their forwarding bands were removed without an
adapter. `Universe` is the state API. The `stores` module is private and does
not re-export `Stores`; that aggregate remains only rollback-coupled
implementation data. `CommandContext` and `InputOpenContext` remain the
borrow-scoped command and host-input capabilities.

The audit retained compact eight-byte node words, coordinated sidecars,
generation and owner validation, survivor pins and refcounts, origin
exclusion, immutable frozen prefixes with mutable overlays, exact state and
format hashes, malformed-reference rejection, group invalidation, dependency
observation, exclusive transaction borrows, and iterative deep-graph
behavior. No hidden forwarding façade, alternate format loader, or test-only
store replay remains.

## Exact cumulative accounting

| Child authority                       | Additions | Deletions |        Net |
| ------------------------------------- | --------: | --------: | ---------: |
| Executable schema (`8394bbbb3`)       |       919 |        63 |       +856 |
| Mechanical migration (`01d37a893`)    |       297 |       514 |       -217 |
| Test replay retirement (`a895bea84`)  |       134 |       976 |       -842 |
| State façade retirement (`84621b1a7`) |        62 |     1,695 |     -1,633 |
| **Authored Rust total**               | **1,412** | **3,248** | **-1,836** |

Documentation and guidance, including the format-authority repair
`070dc6408`, add 75 and delete 20 lines and are not credited as authored-code
reduction. The combined 3,100--4,200-line conditional forecast is short by
1,264--2,364 lines. No moved code, generated source, compact codec, portable
DTO, or documentation is counted as deletion, and no residual reduction is
scheduled.

## Independent verification

With `CARGO_BUILD_JOBS=6`, `tex-state`, the `tex-command`/`tex-exec`
compatibility consumers, and the full routine workspace compiled uncapped.
The source audit confirmed 24 schema descriptors, no retired state façade or
`StoreFormat::restore`, no public `Stores` re-export, and one production frozen
load boundary. Under `MemoryMax=512M`, all three focused schema properties,
19 frozen-name cases, the broader format-filtered cases, both deep iterative
cases, all 729 `tex-state` unit tests plus 11 boundary tests, and all 689
`tex-command` plus 446 `tex-exec` unit tests and their 17 plus 20 integration
tests passed. Under `MemoryMax=1G`, the full routine suite passed. Every
runtime used a finite timeout and `MemorySwapMax=0`; no `prlimit` was used.
Final uncapped `scripts/check.sh` passed all four gates with six Cargo jobs and
both lint resolutions clean across 28 workspace members.
