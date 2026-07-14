# Asynchronous WASM Resource Acquisition

Status: long-term implementation plan. The current restart-on-fetch MVP remains
specified by [wasm_mvp.md](wasm_mvp.md); this document defines its intended
generalization into a typed, batched resource state machine for compilation
inputs and OpenType fonts.

## Goals

The browser frontend must acquire resources only when a session requires them,
without turning dynamic discovery into a serialized network waterfall. Rust
remains synchronous and host-neutral. JavaScript or another host owns
asynchronous I/O, concurrency, persistent caching, cancellation, authentication,
and deployment policy.

The completed design must:

- report every currently knowable missing resource as one deterministic batch;
- distinguish required resources from optional prefetch hints;
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
    pub prefetch_hints: Vec<ResourceRequest>,
}

pub enum SessionAdvance {
    NeedResources(NeedResources),
    Complete(MemoryRunOutput),
    Error(CompileError),
}

pub enum ResourceResponse {
    File(ResolvedFile),
    Font(ResolvedFont),
}
```

Requests are sorted and deduplicated by complete typed identity and contain no
URLs. Responses repeat their request keys, may arrive in any order, and may
satisfy only part of a batch. Another `advance` without any newly satisfied
required request fails with a typed no-progress error.

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
    options?: { signal?: AbortSignal },
  ): Promise<readonly ResourceResponse[]>;
}
```

The facade is an ergonomic driver over `advance` and `provideResources`; it is
not the engine protocol. For each batch it:

1. validates and canonicalizes request keys;
2. forwards required requests to the client resolver;
3. optionally forwards or schedules prefetch hints;
4. validates the iterable response shape and byte limits at the JS boundary;
5. transfers responses into Rust;
6. requires progress on at least one requested resource; and
7. advances again until completion or error.

The application or its resolver decides whether to use memory caches,
in-flight joining, HTTP caching, IndexedDB, a service worker, authenticated
fetches, local user files, or another transport. Reusable helper modules may
implement these policies, but the core package does not require one catalog or
deployment model.

Cancellation aborts work owned only by the cancelled session. A client may
retain a shared in-flight fetch while another live session still references
it. No partially downloaded or partially verified response reaches Rust.

## Prefetch without correctness coupling

Required requests are authoritative. Hints may be absent, incomplete,
overinclusive, stale, or ignored.

A trusted application manifest or format description may hint likely input and
font resources. The coordinator may begin those transfers concurrently. If a
later required request matches a verified cached or in-flight object, it joins
that work. An unused hinted resource never becomes live engine state or enters
HTML output.

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

- unknown or unavailable resource request;
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
7. Add required-versus-hint batching, cancellation, worker transfer, bounded
   partial responses, and no-progress detection.
8. Add long-lived retain/release accounting and client-cache integration hooks
   for incremental render sessions.
9. Remove superseded preload and post-finalization font-delivery APIs after the
   OpenType path covers native, WASM, worker, and browser fixtures.

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
