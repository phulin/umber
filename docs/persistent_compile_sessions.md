# Persistent host compile sessions

This document defines the public compile-session contract shared by native
hosts and the WebAssembly binding. It composes the typed resource protocol in
`wasm_resource_acquisition.md` with the revision and checkpoint rules in
`incremental_v1.md`; it does not introduce a second browser-only engine API.

## One session state machine

`umber::VirtualCompileSession` is the host-neutral session. The WASM
`CompilerSession` is a thin representation adapter over it. A session owns:

- immutable user and resolved resources;
- one accepted root-buffer revision and its incremental checkpoints;
- at most one pending root-buffer patch;
- resource requests discovered while executing the initial or pending
  revision; and
- detached output for the most recently accepted revision.

The existing `advance()`/`compile_attempt()` operation drives every state.
Before the first accepted revision it executes the configured main user file.
After `apply_patch` it executes the pending revision. Either execution may
return `NeedResources`; the caller provides those resources and calls
`advance()` again. Missing-resource execution never accepts or partially
publishes a revision.

`Complete` means that the initial or pending revision was accepted. Calling
`advance()` again without a patch returns the accepted output without
re-executing TeX. Native and WASM callers observe the same result variants.

## Patch contract

A patch is one UTF-8 byte-range replacement against the root editor buffer:

```text
base revision + expected content hash + [start, end) + replacement
    -> monotonically greater next revision
```

The base revision and hash prevent stale edits. Both range endpoints must be
UTF-8 boundaries and must lie within the accepted source. Only one patch may
be pending. A patch is applied atomically when `advance()` accepts it; failure
or a resource request leaves the accepted revision unchanged.

The initial revision is `1`. Public revision numbers use JavaScript-safe
unsigned 32-bit integers at the WASM boundary and widen to the engine's `u64`
identity internally. The accepted revision and content hash are exposed by
the same session so callers do not have to maintain shadow identity state.

## Resource consistency

User files are registered before the initial revision is accepted. The main
file becomes the mutable root editor buffer; included user files and resolved
distribution resources remain immutable for the lifetime of accepted
incremental history. A patch that introduces a new include or font may request
and pin a new immutable resource before acceptance. Rebinding an existing
resource to different bytes remains an error.

Clearing the distribution cache preserves the latest root bytes but discards
accepted incremental history and restarts resource acquisition as a cold
revision.

Resource resolvers are supplied to `tex-incr` execution rather than bypassed
by a browser-specific preflight. This permits initial and patched revisions to
use the identical request keys, virtual paths, font selection, and retry
policy as batch compilation.

## Output and disposal

Every accepted revision returns the existing `CompileOutput` shape, while the
session exposes revision metadata and incremental reuse/retention metrics.
Observable effects are materialized through a lightweight clone of the
accepted retained `World`; inspecting output therefore does not clone engine
stores, consume checkpoints, or prevent later patches.

`dispose()` releases resources, accepted history, and output. No session
method succeeds after disposal.

## Correctness and tests

For each accepted revision, DVI and other observable outputs must be identical
to a fresh cold compile with the same root bytes and pinned resources. Tests
cover initial compilation, multiple patches, resource acquisition introduced
by a patch, stale revisions, hash/range validation, idempotent output reads,
and disposal through both the native and WASM boundaries.
