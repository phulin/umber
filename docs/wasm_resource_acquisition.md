# Asynchronous WASM Resource Acquisition

Status: long-term implementation plan. The current restart-on-fetch MVP remains
specified by [wasm_mvp.md](wasm_mvp.md); this document defines its intended
generalization for compilation inputs and downstream rendering resources.

## Goals

The browser frontend must acquire resources only when a session requires them,
without turning dynamic TeX discovery into a serialized network waterfall.
Rust remains synchronous and host-neutral. JavaScript owns asynchronous I/O,
concurrency, persistent caching, cancellation, and manifest trust. The engine
continues to observe file bytes only through `World`, while HTML fonts remain
downstream driver resources and never enter engine state or page-artifact
identity.

The completed design must:

- report every currently known missing resource as one deterministic batch;
- fetch independent objects concurrently with bounded work and memory;
- use manifest dependencies only as optional prefetch hints;
- resume finalization without rerunning completed TeX execution;
- accept byte-identical duplicate provision idempotently and reject conflicts;
- preserve identical resource validation and HTML bytes in native and WASM;
- make cancellation, corruption, unavailable resources, and no progress typed
  terminal outcomes rather than retry loops; and
- retain immutable resources across incremental render revisions without
  retaining unreferenced assets indefinitely.

## Architectural boundary

The public host protocol is broader than the internal engine file resolver:

```text
                    immutable resource catalog
                      /                  \
        TeX input and TFM objects       HTML font bindings
                   |                            |
              World-backed files          driver cache
                   |                            |
                TeX engine              HTML finalization
```

One JavaScript coordinator may acquire both categories, but Rust must keep
their lifetimes and capabilities separate. A web font is not a virtual file
that the engine may open. Conversely, a TFM supplied to `World` is not by
itself permission to select an unrelated browser face.

## Session protocol

The long-term session interface replaces the file-specific `NeedFiles` result
with an extensible resource batch:

```rust
pub enum ResourceRequest {
    File(FileRequest),
    HtmlFont(HtmlFontRequest),
}

pub struct HtmlFontRequest {
    pub key: HtmlFontKey,
    pub used_codes: CodeSet256,
}

pub enum SessionAdvance {
    NeedResources(Vec<ResourceRequest>),
    Complete(MemoryRunOutput),
    Error(CompileError),
}
```

`CodeSet256` is a canonical 256-bit set. Requests are sorted by their complete
typed identity and contain no URLs. The host resolves identities through its
trusted catalog; TeX input must never construct a fetch URL.

Each `advance` call runs synchronously until it completes, fails, or cannot
continue without resources. `NeedResources` contains the union of every
currently knowable miss. The session accepts responses in any order and may
accept a subset, but another `advance` with no newly satisfied required request
fails with a typed no-progress error.

The initial public response forms are:

```rust
pub enum ResourceResponse {
    File(ResolvedFile),
    HtmlFont(ResolvedHtmlFont),
}
```

Every response repeats its request identity. Registration verifies identity,
declared length, digest, hard limits, and type-specific structure before the
resource becomes visible. Re-registering identical bytes and metadata is a
no-op. Re-registering a different value under an existing identity is a typed
conflict, including at the JavaScript boundary.

## Session states and finalization

The host-neutral session has explicit logical states:

```text
Compiling
AwaitingFiles
Finalizing
AwaitingHtmlFonts
Complete
Failed
```

Missing TeX inputs or TFMs may require another compilation attempt under the
existing restart-on-fetch MVP. Once execution has completed, the session
retains committed artifacts, DVI plans, diagnostics, and staged effects while
HTML resources are acquired. Supplying an HTML font resumes only HTML
finalization; it must not rerun expansion, execution, shipout, or DVI planning.

Before serialization, HTML lowering collects all required font identities and
the union of used codes per identity:

```rust
pub fn collect_html_font_requirements(
    pages: &[PageArtifact],
) -> Result<Vec<HtmlFontRequirement>, HtmlError>;
```

The requirements pass is deterministic, bounded by the existing HTML limits,
and reusable by native callers. It reports all missing bindings in one batch.
The synchronous `HtmlFontResolver` then becomes a lookup over already supplied
bindings, not an acquisition hook.

## Frontend acquisition coordinator

The authored JavaScript facade owns one asynchronous coordinator:

```ts
interface ResourceResolver {
  resolve(
    requests: readonly ResourceRequest[],
    options?: { signal?: AbortSignal },
  ): Promise<readonly ResourceResponse[]>;
}
```

For each batch, the coordinator:

1. canonicalizes and deduplicates the required identities;
2. satisfies memory-cache hits immediately;
3. joins matching in-flight acquisitions instead of issuing duplicates;
4. checks the persistent cache by immutable catalog namespace and digest;
5. expands optional manifest dependency hints;
6. downloads remaining objects with a bounded concurrency pool;
7. verifies response status, length, and SHA-256 before caching;
8. assembles logical resources such as an HTML font binding;
9. provides verified responses to Rust; and
10. calls `advance` again only if the batch made progress.

The default concurrency should be measured rather than enshrined in the wire
protocol. An initial range of eight to sixteen object fetches is appropriate
for browser testing. The resolver must bound queued requests, in-flight bytes,
individual objects, aggregate cached bytes, attempts, and returned output.

Cancellation aborts fetches owned only by the cancelled session. Shared
in-flight acquisitions remain alive while another session is awaiting them.
No partially verified response reaches Rust.

## Prefetch without correctness coupling

Demand requests are authoritative. Manifest dependencies are latency hints and
may be absent, incomplete, or overinclusive.

When a TFM is requested, the standard resolver may begin acquiring the
catalog's associated encoding and WOFF2 objects concurrently with the required
TFM. If the completed artifacts later request that HTML font, the exact request
joins the memory, persistent, or in-flight cache entry. Loading an unused TFM
therefore permits speculative transfer but never causes an unused font to enter
the HTML output.

Format metadata may hint a common family bundle, such as the Plain TeX
Computer Modern set. Concurrent individual content-addressed objects are the
initial strategy. Aggregate downloadable packs should be added only if cold
browser measurements show material request overhead after HTTP/2 or HTTP/3,
bounded concurrency, and persistent caching.

## Catalog and cache identity

The resolver loads one immutable, versioned catalog, following the trust and
canonical-winner model in [wasm_mvp.md](wasm_mvp.md). The catalog namespace and
object digest jointly isolate persistent entries across distributions.

Cache layers are:

- per-session registration, keyed by complete typed resource identity;
- process-wide in-flight and verified-object caches, keyed by object digest;
- optional persistent browser storage, keyed by catalog namespace and digest;
  and
- long-lived render-session retention, reference-counted by live revisions.

TFM identity and WOFF2 object identity are different domains. Several exact
TFM bindings may share one WOFF2 object, and one TFM may be loaded at several
selected sizes without duplicating face bytes.

Persistent eviction is deterministic within an implementation version and
must never delete an entry still referenced by a live session. Cache loss is a
performance event, not a correctness event: the resolver can reacquire an
immutable object from the catalog.

## Integrity, trust, and errors

The catalog origin is the trust root. HTTPS protects transport, while catalog
signatures or deployment policy establish authenticity; content digests alone
detect corruption but do not authenticate a malicious manifest.

Required typed failures include:

- unknown resource identity;
- unavailable catalog object;
- HTTP, CORS, and abort failures;
- declared-length or digest mismatch;
- resource and aggregate limit violations;
- invalid TFM, WOFF2, encoding, or license metadata;
- response type or request-identity mismatch;
- conflicting duplicate registration;
- missing glyph coverage for a used TeX code; and
- retry without progress.

Error messages may include catalog keys and content digests, but not untrusted
markup or executable URLs derived from TeX input.

## Native composition

Native callers use the same request identities, catalog schema, object hashes,
and validation code. A synchronous native adapter may resolve a complete batch
from local files or an application-managed cache, while an asynchronous native
application may drive the same state machine as JavaScript. Equal catalog and
artifact bytes must produce equal HTML and asset bytes.

The existing directory HTML font resolver remains a development and migration
adapter. The manifest-backed catalog becomes the production resolver because a
basename-only directory cannot represent multiple exact identities safely.

## Implementation phases

1. Add canonical `ResourceRequest` and `ResourceResponse` types and adapt the
   existing file-only API without changing its behavior.
2. Make duplicate registration idempotent only for byte-identical resources
   and reject conflicts at native and WASM boundaries.
3. Add the bounded HTML font-requirements pass and retain completed execution
   state across resource-dependent finalization.
4. Move `HtmlFontResolver` behind the supplied-resource cache and return one
   batched HTML font request.
5. Generalize the authored JavaScript retry loop into the concurrent resource
   coordinator with cancellation and no-progress detection.
6. Add immutable catalog font bindings, dependency hints, and persistent cache
   support using the bundle format in
   [web_font_bundles.md](web_font_bundles.md).
7. Add long-lived resource retention and release to incremental render
   sessions.
8. Deprecate direct preload helpers after the catalog path covers the packaged
   Computer Modern fixture and custom bindings.

Each phase keeps the file-only facade working until its replacement is tested
in Firefox, Chromium, Node, and native integration tests.

## Exit criteria

The design is complete when:

- one session result can request multiple files and HTML fonts without ordered
  one-object retry rounds;
- the frontend fetches independent misses concurrently and coalesces duplicate
  in-flight work;
- completed TeX execution is not repeated while awaiting HTML fonts;
- a TFM dependency hint can hide a later exact font request without becoming a
  correctness requirement;
- a warm persistent-cache run downloads no already verified font objects;
- cancellation, corruption, conflict, unavailable-resource, limit, and
  no-progress cases have native and browser coverage;
- native and WASM produce byte-identical HTML from identical artifacts and
  catalog resources; and
- browser latency tests demonstrate that the common Plain TeX path has no
  serialized web-font waterfall.
