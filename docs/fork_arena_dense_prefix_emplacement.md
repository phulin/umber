# Dense fork-arena prefix emplacement

This note records the blocked design decision for `umber2-66p0.8.40.113`.
The required node representation is a dense append-only initialized prefix:
rollback truncates that prefix, lineage retirement clears it or drops whole
chunks, and no interior vacancy or per-slot `Option<Node>` occupancy is
permitted. Engine crates must remain safe Rust.

## Exact evidence

The authenticated 50-million-command baseline from
`umber2-66p0.8.40.110` used binary
`26c457895af9cc9dea71a46b8254a6664f837da6d51fcc3b38c2262dfbddf8a1`.
Its public-copy totals were 13,581,465 `memcpy` calls and 2,026,475,309
bytes plus 191,437 `memmove` calls and 32,922,922 bytes. The largest
application bin was 3,143,705 calls of 168 bytes in
`ChunkStorage::release_lineage`, or 528,142,440 bytes.

A dense `Vec<Node>` prototype proved the desired release behavior. The focused
one/4,096-node gate retained exact first and last values, preserved a shared
lineage, dropped every exclusive value exactly once, reclaimed every chunk,
rejected stale coordinates, reused chunk slots with fresh generations, and
performed zero allocations during release. Exact public-copy attribution found
no `release_lineage` bin and no 168-byte release copy or move.

The same prototype failed the whole-run copy criterion. Straight
`Vec::push` produced 17,324,345 `memcpy` calls and 2,609,483,201 bytes plus
35,652 `memmove` calls and 6,650,435 bytes: 556,735,405 more public-copy
bytes than the baseline. The new leading bins were 3,194,409 and 3,016,158
whole-node appends of 168 bytes. `extend(once_with)` still added 528,166,300
combined bytes. Inlining the entire append and clone path reduced the combined
increase to 67,235,438 bytes, but shifted 441,775,783 bytes from baseline
`memcpy` volume into `memmove`; it therefore also fails the criterion.

Other safe container expressions did not provide emplacement. `resize_with`
emitted both a 168-byte and a 167-byte public copy per focused append.
Appending a provisional valid `Node::Penalty(0)` emitted a 168-byte copy for
the provisional value before the replacement. `SmallVec` emitted the same
whole-node append copies and added spill/construction copies for chunks larger
than its inline capacity. None provides a safe operation that both initializes
`Vec` spare capacity in place and advances its length.

## Design choices

### Reviewed initialized-prefix substrate

A small non-engine crate could own a fixed-capacity
`Box<[MaybeUninit<T>]>` plus one initialized length and expose only safe
operations to engine crates. A consuming `VacantEntry` would commit exactly
one `MaybeUninit::write`, while `truncate`, `clear`, and `Drop` would destroy
only `0..len` and never write a vacancy representation. The crate would
centralize the unsafe `assume_init` and length invariants; `tex-state`,
`tex-exec`, and `tex-incr` would remain unsafe-free.

This representation preserves one allocation per coarse physical page, no
per-node allocation, stable chunk coordinates, chunk-boundary rollback, and
the existing two-lineage ownership model. It requires an explicit exception to
the repository-wide unsafe prohibition, a narrowly reviewed API and invariant
proof, Miri/panic tests for initialization and destruction, and the same exact
whole-run copy census before adoption. No such unsafe boundary is authorized
by the current issue.

### Compact node with arena-owned rare payloads

`Node` could become a compact tag plus common inline fields and coordinates
into dense arena-owned tables for large or rare payloads. Moving a compact
node through safe `Vec` APIs would reduce the measured append volume in
proportion to the new node width, while rare payload tables could remain
chunk-allocated rather than allocating per node.

This is a cross-cutting ownership redesign. Clone, move, semantic identity,
serialization, child dependency floors, page traversal, and destruction would
all need coordinated payload-table rules. It also reduces rather than
eliminates public append copies, so it does not by itself satisfy the present
zero-shift acceptance criterion.

### Accept safe whole-node moves

Keeping the dense `Vec<Node>` prototype preserves the simplest safe ownership
model and gives the desired drop and reclamation behavior. Its exact straight
implementation adds 556,735,405 public-copy bytes to the bounded run.
Code-generation inlining can lower the net increase, but only by converting
hundreds of megabytes to another public copy API and by coupling correctness
code to optimizer behavior. This choice contradicts the current acceptance
criterion and is not suitable for closing the issue.

The issue therefore remains open pending an explicit decision about a reviewed
low-level initialized-prefix boundary or a broader node-layout redesign.
