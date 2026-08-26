# `umber2-7asg.5.10`: Destination-directed raw seams

## Boundary and implementation

Commit `782c5c328` completes the caller migration owned by
`processor/next.rs`, `processor/alignment.rs`, and `command.rs`. Raw resolution
writes into the request's final `Option<CurrentCommand<G>>` and lends that
initialized command back to the driver. Outer validity, alignment
classification, and optional raw observation consequently operate on one
borrowed final value in canonical order. Alignment classification records its
exact committed adjustment on that command instead of returning an
intermediate value for the caller to copy.

The remaining raw-seam alphabetic-constant delivery, ErrorStop deletion,
opening and closing math-shift probes, and unbalanced output-list draining now
pass local command slots to `get_token_into` or `get_x_token_into`. Public
value-returning APIs remain thin boundary conveniences for later scanner and
executor slices; no scoped internal recovery or alignment caller invokes them.

`DeliveryStamp` is still minted after parameter replay and before resolution.
Outer recovery backs up that exact delivery before substituting its space.
Literal braces alone adjust the command-owned `align_state`; backup validates
freshness, consumes the stamp, and reverses the recorded adjustment once.
Delimiter interception still suppresses ordinary raw-command observation.
Observed spelling, canonical identity, origin, and direct source range/location
remain demand-selected borrowed projections of the completed command. The
change adds no cache, heap indirection, destination search, compatibility copy,
input owner, or suspension payload.

Focused coverage in `processor/tests.rs` proves that backup rejects a stale
delivery without applying a second correction, replay mints a fresh stamp, an
alignment transition precedes raw observation, and the raw record carries the
delivered command's exact direct-source provenance.

## Validation

The focused `tex-command` suite passes with 241 unit tests and 17 external
boundary tests. The mixed stored-span, destination-directed, and complete
packed-cutover gates all pass under `flock /tmp/umber-perf-host.lock`; every
warmed allocation row reports zero allocations and zero requested bytes.

The issue-private authenticated control under `target/umber2-7asg.5.10/`
reused the immutable source, distribution, schema-12 format, offline resource
closure, fixed clock, guards, and 20M fuel boundary from the accepted caller
migration controls. The profiling binary has SHA-256
`12ebbabf47b190d705ef21d26508c612db1293b012a503169662bfeafbcfe36c`.
The run intentionally exited 1 at exact fuel exhaustion and reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)`; it used only
issue-private cache and output directories and was serialized by the host
lock.

The complete `cargo test -q --tests` routine suite passes. `scripts/check.sh`
reports all four dprint, Biome, rustfmt, and clippy gates passed.
