# Automatic CTAN Resource Fetch

Status: partially implemented, tracked by the `umber2-mbwq` epic. Builds on the
completed VFS substrate ([umber_vfs.md](umber_vfs.md)) and resource session
protocol ([wasm_resource_acquisition.md](wasm_resource_acquisition.md)). The
shared manifest crate and typed unavailable responses in phases 1 and 2 are
implemented; host fetch, CLI integration, and snapshot deployment remain
planned.

## Problem

A user compiling a real document should not have to install TeX Live or
hand-assemble `.sty`, `.cls`, `.tfm`, format, and font files. Both frontends
should acquire missing distribution files automatically, on demand, from a
CTAN-derived distribution:

- the **web app** already drives the `NeedResources` loop through
  `HttpManifestResolver`, but depends on a deployment-provided manifest and
  hard-fails on any file outside it; and
- the **CLI** (`umber run`, `umber watch`) does not use the resource session
  at all: it searches the local filesystem through `TexInputSearchPath` /
  `TexFontSearchPath` and reports a plain error when a file is absent.

The merged VFS work already provides everything below the host boundary:
typed deterministic request batches, digest and limit validation, idempotent
duplicate registration, conflict rejection, no-progress detection, and atomic
build acceptance, identical in native and WASM. What remains is host-side
acquisition policy and CLI adoption of the session loop.

## Central decisions

### 1. Fetch from a pinned CTAN-derived snapshot, not live CTAN

Raw CTAN is a source archive: many packages publish `.dtx`/`.ins` sources
whose runtime files exist only after a generation step, its directory layout
does not match runtime lookup names, and its contents mutate continuously.
Resolving a `FileRequestKey` against live CTAN would require a
name-to-package index, package unpacking, and in the worst case running
`docstrip` — and would make identical requests yield different bytes over
time.

Instead, both frontends fetch from a **published snapshot**: a pinned,
reproducible, content-addressed object store plus manifest derived from a
distribution tree. This is exactly the model `tools/texlive-wasm-publish`
already implements for the browser: objects named by SHA-256, an ordered
manifest mapping `kind:name` request keys to objects, byte counts, and
dependency hints.

The initial distribution is the **most recent TeX Live snapshot,
self-hosted by this project**: the publisher runs against a current TeX Live
tree (whose runtime files are already generated from CTAN sources), and the
resulting manifest and objects are published to project-controlled hosting.
TeX Live is the upstream of the publisher, not a runtime dependency, and
refreshing the distribution means running the publisher against a newer
snapshot and rotating the default pin.

Consequences:

- one manifest digest pins the complete distribution for a compile, so
  native and web builds of the same document from the same snapshot are
  reproducible and byte-identical;
- the engine keeps its existing guarantee that no Rust code derives a URL
  from a TeX name — hosts map request keys to manifest entries; and
- a later live-CTAN or tlnet backend, if ever wanted, is a new resolver
  implementation behind the same request/response types, not an engine
  change.

### 2. One shared manifest model in Rust

The manifest schema previously existed twice: in the publisher tool and in the
authored JavaScript (`manifest-schema.js`). `crates/umber-distribution` now
owns:

- the manifest data model and strict parser (schema version, distribution
  identity, `objectsBaseUrl`, files, fonts, formats, dependency hints);
- the encoding from `FileRequestKey` / `FontRequestKey` to manifest logical
  keys (today `<kind>:<normalized_name>`), shared with the publisher; and
- selection logic: given a request batch and a manifest, the ordered list of
  objects to acquire (required plus transitive dependency hints) and the
  typed misses.

The crate performs no I/O, has no dependencies, and compiles for
`wasm32-unknown-unknown`. The publisher consumes its model and canonical JSON
serialization; the future CLI fetcher will consume the same API. The
JavaScript implementation remains authored, with fixtures under
`tests/corpus/distribution` asserting that both sides round-trip the same
manifest and select the same ordered jobs and typed misses.

### 3. Distribution absence must be a recoverable engine condition

Today every unsatisfied required request eventually becomes a fatal typed
error (`missing-key` in the web resolver, no-progress in the session). That
is wrong for automatic fetching: LaTeX routinely probes files that are
allowed not to exist (`\IfFileExists`, `\openin` + `\ifeof`, optional `.fd`
and `.cfg` files). With a complete manifest in hand, "not in the
distribution" is an authoritative answer, and TeX's own missing-file
semantics must apply.

Add a negative response to the protocol:

```rust
pub enum ResourceResponse {
    File(ResolvedFile),
    FileUnavailable(FileRequestKey),
    Font(ResolvedFont),
    FontUnavailable(FontRequestKey),
}
```

Registration binds the key to an immutable _absent_ marker in the resolved
layer: the next attempt's resolver reports the ordinary TeX missing-file
condition instead of re-requesting, duplicates are idempotent, and a later
attempt to bind bytes to the same key is a typed conflict. An unavailable
answer counts as progress for no-progress detection. This lands in
`umber-vfs` (`FileProvisioner`), the session resolvers, the WASM wire
encoding, and the JavaScript facade, and replaces the resolver-side
`missing-key` hard failure for requests the manifest does not contain.

### 4. The CLI adopts the session loop with a layered resolver

`umber run` migrates from ad hoc filesystem search to driving
`VirtualCompileSession` exactly as the browser does. The native resolver
answers each `NeedResources` batch through an ordered chain:

```text
project files (main-file directory, TEXINPUTS/TEXFONTS areas)
    -> local persistent object cache (by manifest entry digest)
    -> HTTPS fetch from the pinned snapshot's object store
    -> FileUnavailable
```

Local files win so a document-local `foo.sty` shadows the distribution, the
existing search-order semantics of `TexInputSearchPath` are preserved as the
host policy that produces per-request candidate answers, and everything the
chain returns still passes VFS digest, limit, conflict, and path validation
in Rust. The restricted `|kpsewhich` pipe emulation stays host-side policy.

The cache is a content-addressed store under the platform cache directory
(`$XDG_CACHE_HOME/umber` / `~/Library/Caches/umber`), objects named by
SHA-256, written via temp-file-plus-atomic-rename so concurrent CLI
processes are safe, plus cached manifests keyed by their own digest. Cache
loss is a performance event only; Rust re-verifies every object.

### 5. Networking stays out of engine crates

HTTP lives in one host-side module (in `umber` or a small
`umber-fetch` companion crate) using a blocking client with bounded
per-batch concurrency, timeouts, and retry. `umber-vfs`, the engine crates,
and `umber-distribution` remain free of filesystem, network, and environment
access. The WASM build keeps JavaScript-owned fetch; nothing network-related
compiles into the browser package.

## CLI user model

- `umber run doc.tex` fetches missing distribution files automatically from
  the default pin — the self-hosted TeX Live snapshot — printing one line
  per acquired batch.
- `--distribution <url-or-path>` selects a different snapshot: an HTTPS
  manifest URL or a local manifest path (air-gapped mirrors work by pointing
  at a directory).
- `--offline` (and `UMBER_OFFLINE=1`) answers only from project files and
  the local cache; a required miss is then a distribution-unavailable
  diagnostic naming the exact request keys.
- The snapshot pin ships with the release as a default manifest URL plus
  expected manifest digest; a project may override both. A fetched manifest
  whose digest mismatches its pin is a typed error, never silently used.
- `umber watch` reuses the persistent session: resources resolved once are
  retained across revisions by the VFS resolved layer, so edits never
  refetch, and an in-flight fetch aborts when a newer revision supersedes
  the build.

## Web app model

The browser stack already implements the loop; the work is coverage and
policy, not architecture:

- serve the same self-hosted snapshot (manifest + objects) the CLI defaults
  to, built by the same publisher from the same TeX Live tree, so both
  frontends resolve identical bytes;
- replace the `missing-key` throw with `FileUnavailable` responses;
- forward manifest dependency entries as prefetch hints through the existing
  hint channel so package trees download concurrently rather than as a
  discovery waterfall; and
- keep the existing HTTP/IndexedDB persistent caches as the browser
  equivalent of the CLI object cache.

## Advance-pipeline integration semantics

- **Batching.** The resolver answers one deterministic batch per attempt.
  Required requests are authoritative; dependency hints from the manifest
  are transport-only prefetch and never become engine state unless later
  required.
- **Progress.** Every response — bytes or unavailable — that satisfies an
  outstanding required request is progress. Network failure (HTTP error,
  timeout, abort) satisfies nothing: the CLI surfaces a typed fetch
  diagnostic naming the request keys and object digests rather than looping.
- **Concurrency.** Independent objects in one batch fetch concurrently under
  a host-selected limit; response order and chunking must not affect the
  accepted workspace (already a VFS property test).
- **Cancellation.** Watch-mode revision replacement and Ctrl-C abort the
  fetch layer; no partially downloaded or unverified object reaches the
  session (the existing facade rule, now also enforced natively).
- **Coverage growth.** TeX inputs and TFMs come first (the manifest already
  carries them), then format images (`resolveFormat` already exists in the
  browser; the CLI resolves formats through the same manifest), then
  OpenType fonts per [web_font_bundles.md](web_font_bundles.md) under
  `umber2-y2ei`, then bibliography kinds when `umber2-rti9.12` lands its
  consumers. The request vocabulary and VFS domains already include all of
  these; no protocol change is needed per kind.

## Trust and integrity

- Transport is HTTPS; the release-pinned manifest digest is the trust root
  for selection.
- Every object digest is declared by the manifest and independently
  re-verified by the VFS before registration, in both frontends.
- Hard per-file and aggregate byte ceilings (`VfsLimits`) bound what a
  malicious or corrupt distribution can make the engine retain; the fetcher
  additionally refuses objects whose declared size exceeds the request's
  limit before downloading the body.
- Errors expose request keys, canonical virtual paths, and digests, never
  attacker-controlled markup; URLs appear only in host-side diagnostics.

## Implementation phases

Each phase is a `bd` issue under the `umber2-mbwq` epic (phase N is
`umber2-mbwq.N`); each lands with its tests and keeps
`scripts/check-and-test.sh` and `scripts/check-wasm.sh` green.

1. **Complete — shared manifest crate.** `crates/umber-distribution` owns the
   manifest model, strict parser, canonical serializer, request-key encoding,
   and deterministic job/miss selection. `tools/texlive-wasm-publish` uses it,
   and shared Rust/JavaScript fixtures cover round trips, transitive hints,
   cycles, duplicate requests, and typed file/font misses.
2. **Complete — unavailable responses.** `FileUnavailable`/`FontUnavailable`
   flow through `umber-vfs`, the session, the WASM wire types, and the JS
   facade. Immutable negative bindings provide idempotence, conflict, progress,
   and TeX missing-file semantics, and web manifest misses now produce typed
   negative responses instead of `missing-key` failures.
3. **Native cache and fetcher.** Implement the content-addressed cache and
   blocking HTTP fetch layer with bounded concurrency, atomic writes,
   digest verification, and typed failures; contract-test against a local
   fixture HTTP server, including corruption, truncation, 404, and
   concurrent-process races.
4. **CLI session migration.** Drive `umber run` through
   `VirtualCompileSession` with the layered resolver chain, `--distribution`,
   `--offline`, and pin verification. Existing local-only invocations must
   behave identically (same outputs, same diagnostics) when every file
   resolves locally.
5. **Watch and cancellation.** Reuse the retained session in `umber watch`,
   abort superseded fetches, and verify no refetch across accepted
   revisions.
6. **Publish and adopt the self-hosted snapshot.** Run the publisher against
   the most recent TeX Live snapshot, publish the manifest and objects to
   project-controlled hosting, point the CLI default pin and the web app
   deployment at it, and forward dependency prefetch hints in the browser.
7. **Parity gate.** One corpus document requiring distribution packages
   compiles from a cold cache natively and in the browser fixture to
   byte-identical DVI, satisfies repeat runs entirely from cache, and
   passes an offline-mode run after warming.

## Exit criteria

- `umber run` on a document using distribution packages succeeds on a clean
  machine with no TeX installation, and a second run performs zero network
  requests.
- Native and web builds against the same snapshot pin produce byte-identical
  generated files and DVI.
- A file absent from the distribution produces TeX's own missing-file
  behavior, not a session-fatal error; optional-file probes work.
- Offline mode is fully deterministic: cache-satisfiable builds succeed,
  others fail with typed diagnostics naming exact request keys.
- No engine crate gained network, filesystem, or URL-derivation behavior;
  the JS facade still owns acquisition only.
- Fetch failures, corrupt objects, oversized objects, mismatched manifests,
  and cancellation are typed, tested, and leak no partial state into the
  VFS.

## Open questions

- **Hosting details.** Self-hosting the snapshot is decided; the concrete
  object store/CDN, its bandwidth budget, and the refresh cadence for
  rotating to newer TeX Live snapshots remain deployment decisions. The pin
  mechanism assumes only stable HTTPS URLs, and old snapshots must stay
  available as long as released CLI versions pin them.
- **Local TeX Live probing.** Whether the CLI should optionally probe an
  existing `kpsewhich`-discoverable installation before the network. Default
  answer is no — it reintroduces machine-dependent bytes — but a
  `--texmf <dir>` escape hatch mapping a local tree as a manifest-less
  source may be worth adding for development.
- **Per-project pins.** Whether a checked-in project file should record the
  snapshot pin (lockfile-style) so collaborating users and CI resolve the
  same distribution without flags.
