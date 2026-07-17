# Automatic CTAN Resource Fetch

Status: partially implemented, tracked by the `umber2-mbwq` epic. Builds on the
completed VFS substrate ([umber_vfs.md](umber_vfs.md)) and resource session
protocol ([wasm_resource_acquisition.md](wasm_resource_acquisition.md)). The
shared manifest crate, typed unavailable responses, native cache/fetch layer,
CLI integration, sharded R2 publication, and native production pin are
implemented. Browser sharded resolution remains separate rollout work.

## Problem

A user compiling a real document should not have to install TeX Live or
hand-assemble `.sty`, `.cls`, `.tfm`, format, and font files. Both frontends
should acquire missing distribution files automatically, on demand, from a
CTAN-derived distribution:

- the **web app** already drives the `NeedResources` loop through
  `HttpManifestResolver`, but depends on a deployment-provided manifest and
  hard-fails on any file outside it; and
- the **CLI** (`umber run`, `umber watch`) drives the same resource session,
  with local project search ahead of its pinned distribution resolver; and

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
implements: objects named by SHA-256 and a
[sharded manifest](distribution_manifest.md) whose pinned root transitively
authenticates sorted `kind:name` index shards, object metadata, and complete
inline dependency hints.

The initial distribution is the **most recent TeX Live snapshot,
self-hosted by this project**: the publisher runs against a current TeX Live
tree (whose runtime files are already generated from CTAN sources), and the
resulting manifest and objects are published to project-controlled hosting.
TeX Live is the upstream of the publisher, not a runtime dependency, and
refreshing the distribution means running the publisher against a newer
snapshot and rotating the default pin.

Consequences:

- one small root-manifest digest pins the complete distribution for a compile, so
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

The implemented native boundary is `crates/umber-fetch`. `ObjectCache` stores
objects and manifests in separate digest-keyed namespaces, re-verifies every
read, discards corrupt entries as cache misses, and publishes verified bytes
with same-directory temporary files plus no-clobber atomic persistence.
`umber` exposes this native CLI resource surface and depends on `umber-fetch`
only outside `wasm32`, so the browser build retains the shared compile-session
protocol without compiling the blocking native HTTP client.
`FetchClient` accepts manifest `ObjectEntry` values paired with request keys
and per-request limits. It enforces HTTPS (with loopback HTTP only for
hermetic contract tests), rejects oversized declarations before issuing a
request, bounds response reads, retries transient transport/status/integrity
failures, observes cooperative cancellation during bounded response reads and
before cache/session publication, and returns a batch only if every request
succeeds. Manifest acquisition observes the same cancellation token. Verified peer
downloads may still warm the cache when a batch fails, but no partial response
is exposed to the compile session.

The native resolver treats schema-v2 index shards as digest-keyed manifest
cache entries. It loads only the canonical shard for each unresolved file key,
validates shard distribution/index identity and partition membership, and
treats a missing key in that verified shard as `FileUnavailable`. Schema v2
does not publish logical OpenType font entries, so a verified root answers
those requests as `FontUnavailable`. Complete inline dependency metadata is
sent straight to the object fetch batch without consulting the dependency's
own shard. A batch is still published to the VFS only after every required and
hint object succeeds.

### 6. Snapshots are immutable Cloudflare R2 prefixes

Production snapshots live in a public Cloudflare R2 bucket behind a custom
HTTPS domain. The release tool accepts that public prefix rather than embedding
a provider hostname: a snapshot named `texlive-YYYY` is stored below
`<public-prefix>/texlive-YYYY/`, its manifest points to the sibling `objects/`
prefix, and both the CLI and web deployment consume that exact manifest URL.
The project-controlled production origin is `https://assets.umber.ink/`.
The managed `r2.dev` URL is suitable only for provisioning checks because it is
rate-limited; the release pin must use the custom domain.

R2 was selected because content-addressed objects map directly to immutable
keys and Internet egress is not billed. The operational budget is storage plus
Class A publication and Class B read requests. The custom domain must cache
both JSON and digest-named objects; publications attach a one-year immutable
cache policy. Browser GET/HEAD access uses the bucket CORS policy in
`scripts/texlive-r2-cors.json`.

`scripts/build-texlive-snapshot.sh` is the production staging entry point. It
publishes every runtime-requestable file below TeX Live's `tex/` tree, TFM
metrics, maps, encodings, virtual fonts, and Type 1/OpenType/TrueType/PK/AFM
font areas, plus the Umber-native LaTeX format. TeX Live documentation and
source trees are excluded. The pinned `texlive.tlpdb` supplies runfile package
ownership and direct package dependencies; the publisher emits bounded peer
and cross-package prefetch hints so common package closures start concurrently
without allowing one large package to exceed client resource limits.
Production inventory floors reject seed-sized output.

Publication hands the completed staging directory to `rclone` configured for
R2. Object upload uses bounded transfers and an immutable snapshot prefix;
re-running the command is the supported resumable, idempotent recovery path.
The manifest is uploaded only after every digest-named object, then its digest,
CORS response, and representative objects are verified through
`https://assets.umber.ink/`. Bucket creation, CORS configuration, and custom-
domain activation remain explicit account operations outside the staging
builder. Do not introduce a custom Worker or multipart upload service for this
path.

The production command is `scripts/publish-texlive-r2.sh`. Its checked-in
defaults pin the verified 8-bit-sharded `texlive-2026-r79639` staging bundle,
bucket `umber-assets`, public origin `https://assets.umber.ink`, 154,153
objects, 3,672,643,852 object bytes, and `manifest-v2.json` SHA-256
`7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0`.
The ignored repository `.env` must contain `CLOUDFLARE_ACCOUNT_ID`,
`R2_ACCESS_KEY_ID`, and `R2_SECRET_ACCESS_KEY`; the latter two are the R2 S3
access-key pair, not a Wrangler API token. The script parses only those exact
dotenv keys, passes them through rclone's process environment, and neither
prints them nor creates a persistent rclone config.

Run `scripts/publish-texlive-r2.sh --dry-run` first, then rerun without
`--dry-run` for publication or after any interruption. The command uses
`rclone copy`, never `sync`, so it does not delete older release prefixes or
extra remote keys. It bounds transfers, checkers, and retries; refuses to
overwrite a conflicting digest key; checks every staged object against the
remote; and requires exact object count and byte totals before the first
manifest write. It then fetches the public manifest and three deterministic
objects from the first, middle, and last digest-name positions and verifies
their digests and CORS headers. Bucket creation, the checked-in
`scripts/texlive-r2-cors.json` policy, custom-domain attachment, and credential
creation are one-time Cloudflare account operations outside this script.

Refresh after each annual TeX Live release, or earlier for an urgent corrected
snapshot. A refresh always uses a new snapshot identifier and updates both the
CLI URL/digest constants and the web deployment in the same release change.
Published prefixes are never overwritten, lifecycle-expired, or deleted while
any released CLI version pins them. The bucket must therefore have no deletion
lifecycle rule. Rollback means restoring the previous URL and digest, not
mutating objects.

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
- `umber watch` accepts the same distribution, trust-pin, and offline controls
  as `run` and reuses the persistent session: resources resolved once are
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
3. **Complete — native cache and fetcher.** `crates/umber-fetch` implements the
   content-addressed cache and blocking HTTPS fetch layer with bounded
   concurrency, atomic writes, digest verification, and typed failures.
   Local fixture-server and child-process contract tests cover success, cache
   reuse and corruption, 404, truncation, oversized declarations and response
   lengths, timeout/retry, concurrency bounds, and concurrent-process races.
4. **Complete — CLI session migration.** `umber run --distribution ...` and
   offline runs drive `VirtualCompileSession` through project search,
   content-addressed cache, and verified distribution acquisition. The CLI
   accepts pinned HTTPS or local manifests, resolves formats through the same
   manifest, reports one progress line per acquired batch, and gives typed
   pin-mismatch and offline-unavailable errors. The release-default URL and
   digest slots are present. Every `umber run` output mode uses this path.
   Completed one-shot sessions expose a consuming accepted-finalization
   boundary, so client-owned PDF lowering, format dumping, profiling, HTML
   asset publication, input receipts, effect commit, and atomic driver-file
   publication retain live-state behavior without granting engine execution
   host I/O access.
5. **Complete — watch and cancellation.** `umber watch` now drives one retained
   native resource session across accepted edits. File polling cancels an
   in-flight manifest/object acquisition when a newer edit appears, discards
   only the unaccepted patch, and retries the newest source against the last
   accepted revision; Ctrl-C uses the same path. Tests verify that a resolved
   distribution file is not reopened or refetched on a later revision and
   that cancelled downloads publish neither bytes nor cache objects.
6. **In progress — publish and adopt the self-hosted snapshot.** The verified
   TeX Live 2026 staging bundle, sharded-root publisher contract, and resumable
   rclone publication tooling are complete. Native and browser shard
   resolution and pin adoption remain separate work; publication performs
   structural, public digest, object, and CORS verification before adoption.
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

- **Local TeX Live probing.** Whether the CLI should optionally probe an
  existing `kpsewhich`-discoverable installation before the network. Default
  answer is no — it reintroduces machine-dependent bytes — but a
  `--texmf <dir>` escape hatch mapping a local tree as a manifest-less
  source may be worth adding for development.
- **Per-project pins.** Whether a checked-in project file should record the
  snapshot pin (lockfile-style) so collaborating users and CI resolve the
  same distribution without flags.
