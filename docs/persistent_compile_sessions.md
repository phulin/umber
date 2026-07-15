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

Domain-qualified file request identity, deterministic file batching, generic
registration, and file limits are owned by `umber-vfs`. The compile session
adds TeX lookup/default-extension policy and combines file requests with font
requests, while the WASM layer only adapts the shared Rust wire values.
Authored JavaScript forwards resolver batches unchanged and retains no request,
path, duplicate, progress, or byte-accounting shadow state. Empty and partial
batches therefore reach the same Rust retry state as native calls; stable Rust
error categories are serialized through direct and worker browser APIs.
`FileProvisioner` also owns the session's layered user and resolved-resource
storage plus its accepted generated layer. Each TeX attempt reads inputs and
TFM files from one immutable stage snapshot; the resolver passes selected
shared bytes through `World` so input identity, provenance, and same-run
pending-output precedence remain unchanged. Successful committed auxiliary
files publish through the same stage transaction in deterministic path order.
A resource request, diagnostic failure, or output-limit failure discards that
stage, so the session retains no parallel byte maps, file-accounting counters,
or partially published generated files.

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
accepted checkpoint and artifact-output bases, adds any later per-page render
maps to `output_bytes`, and refreshes diagnostic bytes and protected budget
overage for a later layout line index. The accepted output's copied metrics do
not change.

`dispose()` releases resources, accepted history, and output. No session
method succeeds after disposal.

## Rendered-source queries

HTML output identifies each page and positioned event with `data-umber-page`
and `data-umber-event`; every page pairs its ordinal with the accepted
`data-umber-revision` and the producing session's OS-random 128-bit
`data-umber-output` identity. A text event also exposes its source character codes,
so a browser can translate a pointer hit into an optional text-unit index.
The native session and `CompilerSession.renderedSourceLocation` expose the
producer- and revision-checked lazy query below. The authored `source-map.js` companion reads
the page identity, revision, and event metadata from canonical HTML and translates DOM
caret offsets into the corresponding text unit before making this call:

```text
rendered_source_location(page, event, unit?, dom_output, dom_revision)
    -> Current { revision, path, start, end, line, column }
     | Deleted { minted_revision }
     | StaleRevision { accepted }
     | OutputMismatch { accepted_output }
     | none
```

Pages are numbered from one and events and units from zero. Omitting `unit`
selects the first source-backed unit in the text event, which is sufficient
for coarse run-level navigation. A precise SVG text hit can supply the glyph
or character unit. The caller passes both values stamped on the page. Output
identity is checked first, so HTML from an independent revision-1 session
returns typed `OutputMismatch` before page data is touched. A matching producer
with an old revision returns typed `StaleRevision`, preserving edit invalidation
separately from cross-session misuse.
Invalid page/event/unit values and synthetic output return `none`; they are
not compile errors. An origin whose fragment was removed from the current
editor layout returns typed `Deleted`. While a patch is pending, no query is
served.

The engine does not serialize an eager source map into HTML or page artifact
bytes. Text and math characters plus ligature nodes retain compact
diagnostic-only origin ids through ligaturing, hyphenation, math layout,
packing, and line breaking. Shipout attaches
an in-process origin sidecar aligned with artifact-node preorder, while the
positioned-page lowering records which node and original character produced
each text unit. On the first click into a page, the session parses and
positions that page once, joins event units to the sidecar, and retains only
compact event prefix sums plus opaque origin ids. Later queries are O(1) map
lookups followed by layout-aware resolution. The cache is query-only Rust
state, is discarded on the next accept or rollback, and is never copied into
accepted snapshots. Paths, byte ranges, lines, and columns are computed only
on demand.

The first page query allocates its compact page map, and the first successful
current-document query may allocate the accepted layout's line-start index.
Both remain operational, query-only state: they do not affect semantic
state, snapshot identity, or snapshot capture complexity, and live session
retention telemetry charges them after the query. Ownership determines the
metric: the map is retained with accepted output and increases `output_bytes`;
the line index belongs to the checkpoint layout and increases
`diagnostic_bytes` plus protected checkpoint overage.

Origin columns and artifact sidecars are excluded from semantic node hashes,
artifact bytes, and artifact content identity. Reused committed pages retain
their matching sidecars, and retention metrics charge the sidecar memory.

## Correctness and tests

For each accepted revision, DVI and other observable outputs must be identical
to a fresh cold compile with the same root bytes and pinned resources. Tests
cover initial compilation, multiple patches, resource acquisition introduced
by a patch, stale revisions, hash/range validation, idempotent output reads,
and disposal through both the native and WASM boundaries.
