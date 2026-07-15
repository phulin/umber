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

The retention values copied into an accepted output are a point-in-time
snapshot taken during acceptance. The session's `retention_metrics()` getter,
and therefore the WASM `retentionMetrics` property, is live: it preserves the
accepted checkpoint/output totals but refreshes diagnostic bytes and protected
budget overage so caches allocated by later rendered-source queries are
included.

`dispose()` releases resources, accepted history, and output. No session
method succeeds after disposal.

## Rendered-source queries

HTML output identifies each page and positioned event with `data-umber-page`
and `data-umber-event`. A text event also exposes its source character codes,
so a browser can translate a pointer hit into an optional text-unit index.
The native and WASM sessions expose the same lazy query:

```text
rendered_source_location(page, event, unit?)
    -> { revision, path, start, end, line, column } | none
```

Pages are numbered from one and events and units from zero. Omitting `unit`
selects the first source-backed unit in the text event, which is sufficient
for coarse run-level navigation. A precise SVG text hit can supply the glyph
or character unit. Invalid page/event/unit values and synthetic output return
`none`; they are not compile errors. While a patch is pending, no query is
served, so a returned location always names the same accepted revision as the
rendered HTML.

The engine does not serialize an eager source map into HTML or page artifact
bytes. Text and math characters plus ligature nodes retain compact
diagnostic-only origin ids through ligaturing, hyphenation, math layout,
packing, and line breaking. Shipout attaches
an in-process origin sidecar aligned with artifact-node preorder, while the
positioned-page lowering records which node and original character produced
each text unit. On a click, the session parses and positions only the selected
page, follows that address into the sidecar, and resolves the opaque origin
against the accepted source substrate. Paths, byte ranges, lines, and columns
are therefore computed only on demand.

The first successful current-document query may lazily allocate the accepted
layout's line-start index. That index remains operational, diagnostic-only
state: it does not affect semantic state, snapshot identity, or snapshot
capture complexity, and live session retention telemetry charges it after the
query.

Origin columns and artifact sidecars are excluded from semantic node hashes,
artifact bytes, and artifact content identity. Reused committed pages retain
their matching sidecars, and retention metrics charge the sidecar memory.

## Correctness and tests

For each accepted revision, DVI and other observable outputs must be identical
to a fresh cold compile with the same root bytes and pinned resources. Tests
cover initial compilation, multiple patches, resource acquisition introduced
by a patch, stale revisions, hash/range validation, idempotent output reads,
and disposal through both the native and WASM boundaries.
