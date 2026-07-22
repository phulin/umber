# Asynchronous WASM Resource Acquisition

Status: partially implemented contract and active rollout plan. Typed,
batched file/OpenType resource acquisition and its shared native/WASM retry
state are implemented by the persistent compile session; the remaining
OpenType rollout is tracked by `umber2-y2ei`.

## Goals

The browser frontend must acquire resources only when a session requires them,
without turning dynamic discovery into a serialized network waterfall. Rust
remains synchronous and host-neutral. JavaScript or another host owns
asynchronous I/O, concurrency, persistent caching, cancellation, authentication,
and deployment policy.

The completed design must:

- report every currently knowable missing resource as one deterministic batch;
- distinguish required resources, blocking existence probes, and optional prefetch hints;
- fetch independent objects concurrently under host-selected limits;
- accept OTF/TTF fonts natively and WOFF2 fonts in WebAssembly;
- use one acquired font for engine layout and later HTML output;
- accept byte-identical duplicate provisioning idempotently and reject
  conflicts;
- make cancellation, corruption, unavailable resources, and no progress typed
  terminal outcomes;
- preserve identical validation and semantic resource identities in native and
  WASM; and
- retain immutable resources across render revisions without leaking
  unreferenced objects indefinitely.

## Architectural boundary

The engine reports typed needs but never invokes a resolver:

```text
       client-owned resource policy
     /            |                \
TeX inputs     format images     OpenType fonts
     \            |                /
      verified ResourceResponse batch
                   |
            host-neutral session
            /                  \
       TeX execution       HTML/output reuse
```

Files continue to enter engine I/O through `World`. Fonts enter the immutable
font-program store after OpenType validation and metric projection. In WASM,
the retained WOFF2 bytes also become the browser font asset. No resource name
or TeX input string becomes a URL inside Rust.

## Session protocol

```rust
pub enum ResourceRequest {
    File(FileRequest),
    Font(FontRequest),
}

pub struct NeedResources {
    pub required: Vec<ResourceRequest>,
    pub probes: Vec<ResourceRequest>,
    pub prefetch_hints: Vec<ResourceRequest>,
}

pub enum SessionAdvance {
    NeedResources(NeedResources),
    Complete(MemoryRunOutput),
    Error(CompileError),
}

pub enum ResourceResponse {
    File(ResolvedFile),
    FileUnavailable(FileRequestKey),
    Font(ResolvedFont),
    FontUnavailable(FontRequestKey),
}
```

Requests are sorted and deduplicated by complete typed identity and contain no
URLs. Responses repeat their request keys, may arrive in any order, and may
satisfy only part of a batch. Another `advance` without any newly satisfied
required request or blocking probe fails with a typed no-progress error. A
probe represents `\openin`/existence lookup: the host must answer with verified
bytes or authoritative absence, while actual `\input` remains required.
An unavailable response satisfies its required or probe key for progress purposes and
stores an immutable negative binding. On the next attempt the resolver reports
the ordinary TeX missing-file or missing-font condition without requesting the
key again. Duplicate negative answers are idempotent, while changing a key
between bytes and unavailable is a typed conflict.

File identity and registration are implemented in `umber-vfs`. Every file key
contains a `domain`, `kind`, and normalized relative `name`; the TypeScript
encoding uses the wire names exported by the same Rust enums. File responses
may include `expectedContentId`, the domain-separated VFS identity of the exact
bytes. Native and WASM responses therefore reach the same path, digest, kind,
conflict, and limit validation.

Registration verifies request identity, type, declared length, exact-object
digest when supplied, hard limits, and type-specific structure before making a
resource visible. Re-registering identical bytes and metadata is a no-op.
Registering a different value under an already selected request is a typed
conflict at both native and WASM boundaries.

Font requests and identities follow
[web_font_bundles.md](web_font_bundles.md). The logical request is resolved by
the client. Rust validates the supplied font, computes its immutable program
identity, fixes the session's selection, derives metrics before layout, and
records the exact instance identity in artifacts.

The next cross-output migration, including typed mapping/legacy leaf requests,
provider-scoped absence, resolver precedence, and per-output closure planning,
is fixed normatively by [cross_output_fonts.md](cross_output_fonts.md).

## Session states

The logical states are:

```text
Running
AwaitingResources
Complete
Failed
```

Each `advance` call runs synchronously until execution completes, fails, or
cannot continue without resources. A font is acquired when execution first
needs it, before font-dependent layout. Because the selected font object is
retained, HTML generation reuses it and does not introduce a distinct
post-layout font-finalization state.

Format images are subject to the same ordering rule. An
`OpenTypePreferred` session fails before executing document input when a
loaded format contains classic-only font records; it never attaches a web font
to an already laid-out classic artifact. Clients may run such formats under
the explicit `ClassicTfmExact` compatibility policy, without modern HTML.

The initial file MVP may still restart compilation after a file miss while the
general session is introduced. The completed architecture resumes from the
appropriate retained session boundary and never repeats completed work merely
to reuse a font in output.

## Frontend acquisition coordinator

The authored JavaScript facade may accept a client implementation:

```ts
interface ResourceResolver {
  resolve(
    requests: readonly ResourceRequest[],
    options?: { signal?: AbortSignal; probes?: readonly ResourceRequest[] },
  ): Promise<readonly ResourceResponse[]>;
}
```

The facade is an ergonomic driver over `advance` and `provideResources`; it is
not the engine protocol. For each batch it:

1. forwards Rust-serialized required requests to the client resolver;
2. forwards blocking probes separately and optionally schedules prefetch hints;
3. checks only that the resolver returned an iterable transport batch;
4. transfers the complete batch, including empty and duplicate responses, into
   Rust without maintaining a JavaScript request or path registry; and
5. advances again until the shared session reports completion or a typed
   error, including retry without progress.

The WASM adapter parses the wire representation into the same Rust request
keys and responses used by native callers. `umber-vfs` owns file path,
identity, duplicate, conflict, limit, partial-batch, and progress semantics.
The discriminated wire union uses `file-unavailable` and `font-unavailable` for
negative answers; those variants carry only their complete request key.
The facade therefore has no file-kind table, path canonicalizer, duplicate
map, or resource-byte counter that could drift from native behavior.

The application or its resolver decides whether to use memory caches,
in-flight joining, HTTP caching, IndexedDB, a service worker, authenticated
fetches, local user files, or another transport. Reusable helper modules may
implement these policies, but the core package does not require one catalog or
deployment model.

Cancellation aborts work owned only by the cancelled session. A client may
retain a shared in-flight fetch while another live session still references
it. The facade checks cancellation again after acquisition and before batch
transfer, so no response from cancelled work reaches Rust. No partially
downloaded or partially verified response reaches Rust. The one-shot facade
then disposes the WASM session, and worker timeout or abort terminates the
worker. Direct persistent clients may call `cancelPendingPatch()` before
applying a superseding edit; this drops the suspended candidate and its charged
private engine state while preserving the accepted revision. A later attempt
therefore cannot resume a cancelled suspension.

## Prefetch without correctness coupling

Required requests and probes are authoritative. Hints may be absent, incomplete,
overinclusive, stale, or ignored.

A trusted application manifest or format description may hint likely input and
font resources. The coordinator may begin those transfers concurrently. For a
validated format input closure, the session authorizes positive file responses
for the exact emitted hints and installs the response batch through the same
atomic VFS validation as required files. Other speculative dependencies remain
cache-only. Missing hints never create unavailable bindings, and retry progress
depends only on required requests and probes.

Aggregate packs are a client transport optimization. They do not create a new
engine identity or allow one response to satisfy a mismatched typed request.

## Cache and identity

Cache layers may include:

- per-session registered resources keyed by complete request and selected
  identity;
- process-wide verified objects keyed by exact object digest;
- optional persistent browser objects keyed by application namespace and
  digest; and
- long-lived render-session references to selected immutable resources.

Files, font transport objects, and decoded font programs have separate identity
domains. OTF/TTF and WOFF2 objects may have different byte digests while
decoding to the same canonical font-program identity. Cache loss is a
performance event, not a correctness event: the application may reacquire the
resource and Rust validates it again.

Persistent eviction policy belongs to the application. It must not delete an
object still referenced by a live session. Rust independently enforces hard
per-resource, decoded-font, session-cache, and aggregate-output limits.

The packaged HTTP manifest resolver consumes the schema-2 sharded TeX Live
catalog. Construction from HTTP requires an explicit root SHA-256 pin. Selected
shards are digest-addressed immutable objects and use the resolver's existing
HTTP or IndexedDB cache, while complete inline dependency records prefetch
payloads without a second index lookup. Only verified absence in the canonical
shard produces `file-unavailable`; shard transport and verification failures
remain actionable resolver errors.

PDF virtual-font discovery uses the ordinary file-response loop after engine
execution reaches a retained candidate. Its `vf`, `font-map`, `font-encoding`,
and `font-program` wire kinds resolve through authenticated `tex:<name>` shard
entries, but the resolver returns the original semantic kind rather than the
manifest transport kind. Recursive local metrics remain `tfm` requests. This
keeps native and WASM retry identity identical while preserving the manifest
schema.

Format startup is a distinct, pre-session acquisition. The worker compares an
inline format entry's `engineVersion` and `formatSchema` with the WASM exports
before downloading it, then passes the verified object as a `Uint8Array` to the
shared session without decoding or reformatting it in JavaScript. The Rust
schema-10 decoder performs the complete checksum, compatibility-fingerprint,
directory, fixed-width section, and cross-reference validation; browser code
must not interpret fields using JavaScript or host object layout. The packaged
Plain image and `plain-format.json` are regenerated together from
`plain-source.lock`, and browser acceptance tests cover both schema mismatch
before acquisition and checksum failure after byte transport.

A schema-3 format entry may also carry a validated schema-1 input closure. The
worker converts those canonical file keys to the ordinary typed request wire
form and supplies them with the format bytes. Rust sorts, deduplicates, bounds,
and emits them exactly once as `prefetchHints` on the first real resource miss.
The JavaScript resolver warms verified shards and objects concurrently and
returns positive responses for top-level closure hints, but not their
transitive dependency prefetches. Rust has already removed registered and user
inputs from the authorized set; the resolver omits absent, failed, and
over-budget speculation. Hints therefore cannot override user inputs, create
unavailable bindings, or affect retry progress.

## Client-owned distribution

The client maps logical requests to resources. Valid implementations include:

- application-bundled URLs;
- a client JSON manifest;
- a CDN keyed by content digest;
- a separate Computer Modern asset package;
- local user-selected fonts;
- private authenticated storage; and
- a service-worker-managed offline distribution.

The core API neither specifies nor consumes those catalog formats. It does not
derive an object path from a request name. The client may declare an expected
digest or canonical font-program identity, but Rust verifies every declared
value.

## Native composition

Native callers drive the same request and response types. A synchronous adapter
may resolve a batch from explicit local paths or application assets; an
asynchronous native host may use the same loop as JavaScript. Native fonts use
OTF or TTF containers initially, while WASM fonts use WOFF2. Equivalent
containers must produce the same canonical font-program identity and metric
projection.

Native filesystem lookup is an application policy. The engine does not search
system font directories or infer a file path from a logical font name.

## Integrity, trust, and errors

The client-selected resource source is the trust root for selection and
licensing. HTTPS, signed manifests, application bundling, or deployment policy
may establish authenticity. Content digests detect corruption but do not
authenticate a malicious catalog.

Required typed failures include:

- unexpected resource responses and conflicting availability bindings;
- HTTP, CORS, authentication, and abort failures reported by the client;
- declared-length or object-digest mismatch;
- resource and aggregate limit violations;
- invalid file, format, OTF, TTF, collection, or WOFF2 structure;
- canonical font-program identity mismatch;
- response type or request-identity mismatch;
- conflicting duplicate registration;
- unsupported font face, variation, mapping, shaping, or math data;
- missing glyphs required by the document; and
- retry without progress.

Errors may contain logical request keys and content digests, but not untrusted
markup or executable URLs derived from document input.

## Implementation phases

1. Generalize the file-only result into canonical `ResourceRequest`,
   `ResourceResponse`, `NeedResources`, and `SessionAdvance` types without
   changing existing file behavior.
2. Make duplicate provisioning idempotent only for identical resources and
   reject conflicts in Rust, WASM, worker, and JavaScript facades.
3. Add the OpenType font request and response types defined by
   [web_font_bundles.md](web_font_bundles.md).
4. Acquire and validate fonts before layout, retain them by canonical program
   identity, and record selected instance identities in artifacts.
5. Reuse retained font objects directly in embedded and manifest HTML output.
6. Expose the low-level `advance`/`provideResources` API and drive it through an
   optional high-level client resolver facade.
7. **Complete.** Add required-versus-hint batching, cancellation-facing
   disposal, worker transfer, bounded partial responses, and shared Rust
   no-progress detection. JavaScript now owns acquisition only and forwards
   response batches without duplicating path, identity, or lookup semantics.
   The same facade selects `ProjectSession` for bibliography project options;
   TeX, bibliography, convergence, diagnostics, and generated-file publication
   remain inside Rust while browser, Node, and worker callers retain the same
   acquisition-only resolver contract.
8. Add long-lived retain/release accounting and client-cache integration hooks
   for incremental render sessions.
9. Superseded preload and post-finalization font-delivery APIs were removed
   after the OpenType path covered native, WASM, worker, and browser fixtures.
   Legacy mapping, embedding permission, provenance, and WOFF2 now arrive
   atomically in the typed font response.

## Exit criteria

The resource layer is complete when:

- one session result can request multiple files and fonts without ordered
  waterfalls;
- required requests and hints have distinct correctness semantics;
- native and WASM validate equivalent resources to identical semantic
  identities;
- WOFF2 supplied before WASM layout is reused for HTML without another fetch;
- response order and chunking do not change results;
- identical duplicate provisioning is a no-op and conflicts fail atomically;
- cancellation and no-progress paths terminate without leaking resources;
- retained-resource counts return to baseline after session and revision
  disposal;
- client-owned static, manifest, CDN, private, and persistent-cache resolvers
  pass the same contract tests; and
- no engine or package path derives a URL from a document resource name.
