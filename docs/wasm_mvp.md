# Browser WebAssembly MVP

Status: proposed design.

This document specifies the smallest useful browser-facing WebAssembly build
of Umber. The MVP accepts a main TeX source plus optional user files, acquires
missing TeX Live inputs and TFM files from an HTTP-hosted distribution, and
returns terminal output, DVI bytes, and committed auxiliary output files.

The design preserves the existing architecture: engine state stays in
`Universe`, external facts stay behind `World`, and host policy stays in a
driver. Browser networking does not enter `tex-lex`, `tex-expand`, `tex-exec`,
or `tex-state`. See [architecture.md](architecture.md) and
[core_state.md](core_state.md).

## 1. Goals and non-goals

### Goals

1. Publish an ES-module package that can be initialized with `wasm-pack`
   output and called from a browser or web worker.
2. Provide one asynchronous JavaScript `compile` operation even though the
   engine and its file-open hooks remain synchronous.
3. Load user files from memory and lazily download distribution `.tex` and
   `.tfm` files through a caller-supplied resolver.
4. Batch every missing file discovered in one engine attempt, cache successful
   downloads, and retry deterministically until the job completes or reaches a
   bounded failure.
5. Preserve TeX input precedence: user files override distribution files, and
   the hosted distribution chooses one canonical TeX Live result for each
   logical lookup key.
6. Produce the same semantic engine behavior as a native memory-backed run for
   the same bytes, format image, job clock, and options.
7. Make the protocol testable without a browser or live network.

### Non-goals

- Resuming an `Executor` at the exact instruction that requested a file. The
  MVP starts a fresh memory-backed run after each fetch round.
- Discovering the exact transitive file set before executing TeX. A missing
  `\input` blocks discovery of dependencies named by that file.
- Loading TeX Live's prebuilt `.fmt` files. They belong to other engines and
  are not compatible with `Universe::from_format`; only Umber-generated format
  images are accepted.
- Remote lazy loading for `\openin`. That primitive currently opens through
  `World` directly, missing probes are valid TeX behavior, and it has no driver
  hook on which to suspend. User-provided and already-cached files remain
  available to `\openin`.
- Shell escape, subprocesses, host filesystem access, interactive terminal
  reads, SyncTeX, virtual fonts, PK fonts, or non-DVI output drivers.
- Perfect Kpathsea emulation in the client. TeX Live precedence is flattened
  into the hosted manifest when the distribution is published.
- Offline installation of an entire TeX Live tree. Persistent browser caching
  is an optional resolver concern, not part of the engine state.

## 2. Why compilation uses fetch rounds

`ExpansionHooks::open_input` and `ExpansionHooks::open_font` are synchronous.
Browser `fetch` returns a promise, and blocking the browser thread until that
promise resolves is neither portable nor acceptable. Static source scanning is
also insufficient because TeX can construct filenames with macros and choose
them through conditionals.

The MVP separates these responsibilities:

- a synchronous Rust `compile_attempt` executes against bytes already cached
  in the session;
- a JavaScript facade awaits the resolver for every returned missing-file
  request; and
- the next attempt creates a fresh `World::memory()` and `Universe`, seeds all
  cached bytes, and executes again from the main file.

This is one asynchronous operation to the application, but it may contain
several engine attempts and HTTP rounds:

```text
application
    |
    | await compile(...)
    v
JavaScript facade -----> compile_attempt() in WASM
    ^                         |
    |                         +-- Complete / Error
    |                         |
    +-- resolver.fetch() <----+-- NeedFiles([...])
            |
            +-- provide resolved bytes to session, then retry
```

A missing TFM file is recoverable in the current stomach: TeX installs
`nullfont` and continues. The WASM driver must therefore record TFM misses as
side state and return `NeedFiles` instead of accepting the apparent successful
run. This often collects several fonts in one attempt. A missing `\input`
raises an expansion error and ends the attempt; the result includes that input
plus every earlier TFM miss. Files referenced only after the missing input are
necessarily discovered in a later attempt.

## 3. Component boundary

The implementation is split into a host-neutral session and a thin binding:

```text
JavaScript/TypeScript facade
  - async resolver calls, HTTP, Cache Storage/IndexedDB, SHA-256 verification
  - retry loop, AbortSignal, download concurrency
                    |
                    v
umber-wasm (new cdylib)
  - wasm-bindgen types and Uint8Array conversion only
                    |
                    v
umber::VirtualCompileSession (new host-neutral driver module)
  - user/distribution byte cache, request bindings, limits, compile attempts
  - WorldInput and ExpansionHooks composition
                    |
                    v
existing tex-exec / tex-expand / tex-lex / tex-state / tex-out pipeline
```

The host-neutral session belongs in `umber`, not `umber-wasm`, so request
classification, retries, output collection, and limit enforcement have fast
native unit tests. The binding crate must not duplicate engine-driver logic.

`umber-wasm` is a workspace crate with library crate types `cdylib` and
`rlib`. Its production dependencies are `umber`, `wasm-bindgen`, and the small
serialization/binding support required by the chosen API representation. It
must not depend on `web-sys`; the authored JavaScript facade owns networking.

The browser target is `wasm32-unknown-unknown`. The adapter enables
`getrandom`'s JavaScript backend for the `ahash` dependency and configures
`getrandom_backend="wasm_js"` only for that target. Native builds retain their
current backend.

## 4. Virtual filesystem and lookup identity

Every compilation uses a POSIX-like virtual namespace independent of the
browser URL space:

```text
/job/                       user-owned files
/job/main.tex               default main file
/texlive/<canonical path>   resolved distribution files
```

Paths are normalized before entering a session. Normalization converts `.`
segments, rejects `..`, NUL, backslash, URL syntax, and paths outside the two
roots, and applies the default extension before constructing a lookup key.
Browser URLs never serve as engine paths.

The session stores two kinds of entries:

```rust
struct UserFile {
    virtual_path: VirtualPath,
    bytes: Vec<u8>,
}

struct ResolvedFile {
    request: FileRequestKey,
    virtual_path: VirtualPath,
    bytes: Vec<u8>,
}
```

`UserFile` lookup has priority. `ResolvedFile` binds a logical request to the
canonical virtual path selected by the distribution manifest. This binding is
needed because the current CLI search path accepts an ordered list of
directories, while a full TeX Live tree contains recursive directories and
duplicate basenames. The browser driver does not probe arbitrary CDN paths.

The two initial remote kinds are:

```rust
enum FileKind {
    TexInput,
    Tfm,
}

struct FileRequestKey {
    kind: FileKind,
    normalized_name: String,
}
```

The key is structural, not a concatenated string internally. The manifest
encoding uses `tex:<name>` and `tfm:<name>`. A request also carries the original
spelling for diagnostics, but identity and deduplication use the normalized
name. Explicit subdirectories remain part of that name.

## 5. Compile-attempt algorithm

`VirtualCompileSession::compile_attempt` performs these steps:

1. Check session limits before allocating engine state.
2. Construct a deterministic `World::memory_with_clock` using the clock in the
   session options.
3. Seed every user file and resolved distribution file into the memory world
   at its virtual path.
4. Construct `Universe` from the optional Umber format image. If no image is
   supplied, construct a fresh universe and call `prepare_run_stores`.
5. Read the main file through `World`, construct `WorldInput`, and create an
   `InputStack`.
6. Run `Executor` with a `VirtualRunHooks` value that owns an ordered,
   deduplicated missing-request collection.
7. For `\input`, check the user area and existing resolved binding. If neither
   is available, record `TexInput`, return the ordinary hook error, and allow
   expansion to terminate the attempt.
8. For a TFM open, perform the corresponding checks. If unavailable, record
   `Tfm` and return an error to the existing font assignment path. Execution
   continues with `nullfont`, allowing more missing fonts to be collected.
9. If any request was recorded, discard all attempt-local engine state and
   return `NeedFiles`, even if execution otherwise completed or later produced
   another diagnostic. The absent file may change that later behavior.
10. If no request was recorded and execution succeeded, commit the remaining
    World effect prefix exactly as the native CLI does. Collect terminal, log,
    and auxiliary output bytes only after this final commit, assemble DVI bytes
    from the committed page artifacts, and return `Complete`. If execution
    failed without a miss, return the real engine diagnostic instead.

Retries must satisfy a progress invariant: at least one previously unavailable
request must receive a valid binding before another attempt begins. If the
resolver returns no progress, the facade fails instead of looping.

All attempt-local terminal text, log bytes, artifacts, and output stream bytes
are discarded with a `NeedFiles` result. Only the final no-miss attempt is
observable to the application.

## 6. Rust and JavaScript APIs

The exact wasm-bindgen representation may use exported classes or
`serde-wasm-bindgen`, but the generated TypeScript surface must be equivalent
to the following.

### Low-level synchronous WASM API

```ts
export class CompilerSession {
  constructor(options: SessionOptions);
  addUserFile(path: string, bytes: Uint8Array): void;
  provideResolvedFile(
    request: FileRequestKey,
    virtualPath: string,
    bytes: Uint8Array,
  ): void;
  compileAttempt(): AttemptResult;
  clearDistributionCache(): void;
}

export interface SessionOptions {
  mainPath: string;
  jobName?: string;
  format?: Uint8Array;
  clock?: { year: number; month: number; day: number; minutes: number };
  limits?: Partial<SessionLimits>;
}

export interface SessionLimits {
  attempts: number;
  resolvedFiles: number;
  oneFileBytes: number;
  cachedFileBytes: number;
  userSourceBytes: number;
  outputBytes: number;
}

export type AttemptResult =
  | { kind: "need-files"; files: FileRequest[] }
  | { kind: "complete"; output: CompileOutput }
  | { kind: "error"; diagnostic: Diagnostic };

export interface FileRequestKey {
  kind: "tex" | "tfm";
  name: string;
}

export interface FileRequest extends FileRequestKey {
  originalName: string;
}

export interface CompileOutput {
  terminal: string;
  log: Uint8Array;
  dvi: Uint8Array;
  files: Array<{ path: string; bytes: Uint8Array }>;
}

export interface Diagnostic {
  message: string;
  file?: string;
  line?: number;
  column?: number;
}
```

Large byte fields must cross as `Uint8Array`; they must not be encoded as JSON
number arrays or base64. Ownership rules must be explicit: input arrays are
copied into the session in the MVP, while output getters may transfer/copy once
and then clear their Rust buffers.

`clearDistributionCache` keeps user files and options but drops remote bytes
and request bindings. The application may also discard the session to release
all WASM allocations.

### High-level asynchronous facade

```ts
export interface FileResolver {
  resolve(
    requests: readonly FileRequest[],
    signal?: AbortSignal,
  ): Promise<readonly ResolvedDownload[]>;
}

export interface ResolvedDownload {
  request: FileRequestKey;
  virtualPath: string;
  bytes: Uint8Array;
}

export async function compile(
  options: SessionOptions,
  userFiles: ReadonlyMap<string, Uint8Array>,
  resolver: FileResolver,
  signal?: AbortSignal,
): Promise<CompileOutput>;
```

The facade constructs a session, adds user files, calls `compileAttempt`,
awaits `resolver.resolve` on `need-files`, provides the returned bytes, and
retries. It applies the round and byte limits even when a custom resolver is
used. An `AbortSignal` is checked before an attempt, before fetch, and after
fetch; an engine attempt itself is synchronous and cannot be interrupted in
the MVP. `compile` is therefore the worker/local-realm entry and must not be
called on a browser UI thread for untrusted or potentially long-running input.

The package also ships a main-thread controller for its standard HTTP manifest
resolver. The exact bundler URL syntax may vary, but its semantic API is:

```ts
export interface HttpManifestResolverOptions {
  manifestUrl: string;
  persistentCache?: "http" | "indexeddb" | "none";
  concurrency?: number;
}

export function compileInWorker(
  options: SessionOptions,
  userFiles: ReadonlyMap<string, Uint8Array>,
  resolver: HttpManifestResolverOptions,
  control?: { signal?: AbortSignal; timeoutMs?: number },
): Promise<CompileOutput>;
```

The controller starts a dedicated module worker, transfers input buffers,
receives the final output buffers, and terminates the worker after success or
failure. Abort and timeout terminate the worker from the owning realm, which
still runs while WASM is executing synchronously. A custom `FileResolver`
cannot be transparently cloned into a worker; applications using one are
responsible for calling the low-level facade inside their own worker.

## 7. Hosted TeX Live manifest

The standard HTTP resolver loads one immutable, versioned manifest. A minimal
manifest is:

```json
{
  "schema": 1,
  "distribution": "texlive-2026",
  "objectsBaseUrl": "https://cdn.example/texlive/2026/objects/",
  "files": {
    "tex:plain.tex": {
      "virtualPath": "/texlive/tex/plain/base/plain.tex",
      "object": "sha256-<hex>",
      "sha256": "<hex>",
      "bytes": 45231,
      "dependencies": ["tex:hyphen.tex", "tfm:cmr10.tfm"]
    },
    "tfm:cmr10.tfm": {
      "virtualPath": "/texlive/fonts/tfm/public/cm/cmr10.tfm",
      "object": "sha256-<hex>",
      "sha256": "<hex>",
      "bytes": 1296,
      "dependencies": []
    }
  }
}
```

The publisher generates `files` from one pinned TeX Live release and its
ordered TEXMF roots. For each supported kind and logical name, it records the
same canonical winner that the deployment intends Kpathsea to return. Duplicate
logical names are therefore resolved at publication time, not by HTTP probe
order. Manifest generation must reject case-fold collisions and paths that do
not normalize into `/texlive`.

Object names are content-addressed and served with immutable cache headers.
The resolver checks response status, declared byte length, and SHA-256 before
calling `provideResolvedFile`. HTTPS protects transport; the digest detects
corruption and manifest/object mismatches. Authenticity still depends on how
the application trusts the manifest origin.

`dependencies` is an optional performance hint. The standard resolver may
expand the closure and fetch those objects concurrently, but it must bind each
download under its own manifest lookup key. Hints may over-fetch and may be
incomplete; only actual engine requests determine correctness.

The manifest is metadata, not engine input. Its version and resolved object
digests should be returned as optional build provenance, but must not enter
TeX state or pretend to be `World` input records unless their bytes are opened
by the engine.

## 8. Caching and concurrency

The session cache is authoritative for one `compile` call. The standard
resolver may add two browser-side layers:

1. Cache Storage for immutable HTTP responses, relying on content-addressed
   URLs and normal browser HTTP caching.
2. IndexedDB for deployments that need explicit cache quotas, inventory, or
   offline reuse.

Persistent caching is keyed by distribution id plus SHA-256, never only by a
logical filename. Switching manifest versions cannot silently reuse bytes from
another TeX Live release.

The standard resolver downloads a bounded number of objects concurrently
(default 8). It deduplicates identical hashes and lookup keys within a batch.
It may fetch dependency hints in the same round, but requested files have
priority. Failed speculative dependency downloads do not fail the job unless
the engine later requests that file; failed requested downloads do.

## 9. Limits and failure model

The MVP applies conservative defaults, configurable downward or upward only to
the following hard ceilings:

| Limit | Default | Hard ceiling | Purpose |
|---|---:|---:|---|
| Compile attempts | 32 | 128 | Bound dynamic dependency rounds |
| Resolved files | 512 | 4096 | Bound manifest/cache fan-out |
| Concurrent downloads | 8 | 32 | Bound browser/network pressure |
| One file | 16 MiB | 64 MiB | Reject accidental large objects |
| Total cached file bytes | 64 MiB | 256 MiB | Bound WASM memory growth |
| Main/user source bytes | 16 MiB | 64 MiB | Bound initial input |
| All returned output | 64 MiB | 256 MiB | Bound terminal, log, DVI, and aux data |
| Standard worker wall time | 10 s | 60 s | Terminate runaway compilation |

The public error model distinguishes:

- invalid options or virtual paths;
- unsupported or incompatible Umber format images;
- unresolved manifest keys;
- HTTP, CORS, abort, size, and digest failures from the resolver;
- no-progress and attempt-limit failures from the facade;
- worker timeout or termination from the main-thread controller;
- WASM/session memory limits; and
- TeX diagnostics from a no-miss engine attempt.

Diagnostics are data, not thrown Rust panics. The binding converts expected
errors into typed JavaScript errors/results. Panics indicate bugs and are not a
supported recovery mechanism.

User file precedence is also a security boundary: distribution resolution may
not replace a path already supplied under `/job`. Absolute TeX input names are
accepted only when they normalize inside `/job`; absolute distribution paths,
URL-shaped names, and traversal attempts are rejected rather than fetched.
Shell escape remains disabled, and the memory-backed `World` is mandatory.

File and output limits do not bound all transient `Universe` allocations, and
the current expansion loop has no cooperative fuel counter that can interrupt
an infinitely recursive macro from inside one attempt. For untrusted input,
the dedicated worker and owner-enforced timeout are part of the MVP security
boundary, not merely a performance recommendation. An in-process memory limit
is deferred; a browser may terminate a worker that exhausts its available
memory, and the application must treat that as a failed job.

## 10. Required repository changes

### `umber`

- Add host-neutral `VirtualCompileSession`, request/result types, virtual-path
  validation, cache limits, and `VirtualRunHooks`.
- Run the main source as `WorldInput`, not `MemoryInput`, so successful
  `\input` files receive ordinary content-addressed World records and source
  provenance.
- Record typed misses in hook side state. Do not infer missing requests by
  parsing `ExecError` display strings.
- Keep the existing CLI behavior and public `TexInputSearchPath` /
  `TexFontSearchPath` behavior unchanged.
- Expose final terminal, DVI, log, and auxiliary outputs through one native
  attempt result.

### `tex-state`

- Add a read-only iterator or copying snapshot for all materialized memory
  output files. The existing `memory_output(path)` remains useful, but a
  browser caller cannot know every aux/toc/idx path in advance.
- Do not expose the `MemoryBackend` map or timeline-control methods.

### `umber-wasm`

- Add the `cdylib`/`rlib` binding crate and browser-target randomness setup.
- Translate host-neutral session methods and results to wasm-bindgen values.
- Preserve binary values as typed arrays and emit useful TypeScript
  declarations.
- Ship the authored async facade and standard manifest resolver alongside the
  generated WASM package.
- Ship the standard module-worker entry and main-thread timeout/abort
  controller; document the synchronous low-level API as unsuitable for
  untrusted work on a UI thread.

The release build is produced with:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build crates/umber-wasm --target web --release
```

The packaged directory contains the optimized `.wasm`, generated low-level ES
module, authored asynchronous facade, TypeScript declarations, manifest
resolver, package metadata, and licenses. The package exposes the authored
facade as its default browser entry and the generated low-level binding under
an explicit advanced entry. A smoke-test page must load the package over HTTP;
opening it through `file:` is not a supported deployment mode.

No pipeline crate needs an asynchronous trait, JavaScript dependency, URL
type, HTTP cache, or browser conditional.

## 11. Verification

### Native tests

- Virtual path normalization and rejection.
- User-file precedence over resolved distribution bindings.
- Default `.tex` and `.tfm` extensions and explicit subdirectory identity.
- Stable request deduplication and ordering.
- One missing input produces `NeedFiles` rather than an engine diagnostic.
- Several missing fonts are batched before completion.
- Earlier font misses plus a later input miss are returned together.
- Providing bytes makes progress and a retry reaches completion.
- Attempt-local effects and artifacts do not leak across retries.
- Format compatibility errors, no-progress detection, and every resource
  limit.
- Memory output enumeration does not expose or mutate backend storage.

### WASM tests

Use `wasm-bindgen-test` in a headless browser for:

- construction and disposal of a session;
- `Uint8Array` input/output fidelity, including embedded zero bytes;
- discriminated attempt results and TypeScript-facing field names;
- repeated attempts without unbounded retained allocations; and
- JavaScript exception conversion for invalid boundary inputs;
- owner-enforced worker timeout for a deliberately nonterminating TeX input.

### Browser integration fixture

A local HTTP server hosts a tiny generated manifest and objects. The fixture
starts with only `main.tex`, then demonstrates at least:

1. one blocking remote `\input` round;
2. one round that batches two TFM files;
3. SHA-256 verification;
4. a final DVI byte stream and terminal output; and
5. a warm-cache second compilation with no HTTP object requests.

Implementation work runs the relevant crate tests explicitly, then
`scripts/check.sh`. The full correctness gate remains
`cargo test --workspace --tests`; fixture regeneration, if needed, remains
through `scripts/regen-fixtures.sh`.

## 12. Delivery sequence

1. Implement and test the host-neutral virtual session and memory-output view.
2. Add `umber-wasm`, target configuration, bindings, and browser binding tests.
3. Add the asynchronous JavaScript facade with an injectable resolver and
   bounded retry loop.
4. Add the versioned-manifest resolver, integrity checking, and local HTTP
   integration fixture.
5. Produce a release package containing the WASM binary, ES module facade,
   TypeScript declarations, license metadata, and a minimal browser/worker
   example.

The MVP is complete when a clean browser session can compile the integration
fixture using lazily hosted `.tex` and `.tfm` objects, a second run can reuse
the browser cache, malformed or oversized inputs fail within the documented
limits, and native behavior remains green under the repository quality gates.

## 13. Deferred evolution

If retry cost becomes material, replace restart-on-fetch with an explicit
driver suspension protocol. That work must define a quiescent file-open
boundary, preserve the executor continuation and input stack without hidden
Rust-stack state, and integrate with the existing resume-valid checkpoint
rules. It must not treat an arbitrary snapshot taken during `\input` scanning
as resumable.

Remote `\openin` can be added separately by routing stream opens through a
narrow driver file-resolution capability. The design must preserve the TeX
distinction between an expected nonexistent probe and a remotely available
file; blindly fetching every failed `\openin` is not correct.

Additional resource kinds (`.vf`, encodings, maps, PK fonts, images) extend
`FileKind` and the manifest only when the corresponding engine subsystem can
consume them through `World`.
