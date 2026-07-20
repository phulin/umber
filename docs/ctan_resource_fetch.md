# Automatic CTAN Resource Fetch

Status: complete under the `umber2-mbwq` epic. Builds on the
completed VFS substrate ([umber_vfs.md](umber_vfs.md)) and resource session
protocol ([wasm_resource_acquisition.md](wasm_resource_acquisition.md)). The
shared manifest crate, typed unavailable responses, native cache/fetch layer,
CLI integration, sharded R2 publication, native and browser production pins,
and the cross-frontend cold/warm/offline parity gate are implemented.

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
own shard. A batch is still published to the VFS only after every required
object succeeds; failed speculative objects are omitted.

Schema-3 roots add format input closures without changing schema-2 behavior.
After the selected pinned format reaches its first actual input miss, the
session forwards its validated closure as a one-shot hint batch. Native and
browser resolvers deduplicate it with required work, enforce the existing file
and byte budgets, warm verified closure objects concurrently, and return
positive responses for the exact top-level closure hints. The session installs
those responses atomically while keeping required-only retry progress. Missing,
stale, oversized, or failed speculative entries are ignored without
unavailable bindings; required acquisition retains its existing typed failure
behavior.

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
  Required requests are authoritative. Exact format-closure hints may become
  validated VFS inputs for the next retry; transitive manifest dependencies
  remain transport-only prefetch.
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

## Verification

The native and browser gates start from empty caches, resolve the same pinned
manifest and objects, and require byte-identical generated files and DVI. Warm
and explicit offline runs must reproduce those bytes without network access.
Missing optional files follow TeX not-found semantics; required misses,
pin/digest failures, corruption, oversize objects, cancellation, and cache
races are typed and may not publish partial VFS state.

`scripts/test-wasm-browser.sh` covers the shared browser/native fixture.
Fetcher contract tests cover bounded concurrency, atomic cache writes,
corruption recovery, truncation, response limits, timeout/retry behavior, and
concurrent processes. Engine crates remain free of networking, filesystem, and
URL-derivation behavior; the authored JavaScript facade owns browser transport.
