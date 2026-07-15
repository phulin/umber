# Shared virtual filesystem

Status: canonical paths, immutable files, layered storage, typed file requests,
resource registration, file limits, and deterministic snapshots implemented;
transactions proposed.

This document defines `umber-vfs`, the host-neutral virtual filesystem shared
by Umber's TeX driver, bibliography processing, native embeddings, and the
WebAssembly binding. It extracts path, immutable-file, resource-registration,
and generated-output ownership from `umber::VirtualCompileSession` without
moving TeX semantics or host acquisition policy into a lower layer.

The central decision is:

> All in-process document stages read immutable snapshots of one virtual
> workspace and publish writes through transactions. User files and acquired
> resources are immutable inputs; generated files form a private build overlay
> that becomes visible to callers only when the complete build is accepted.

The existing resource state machine in
[`wasm_resource_acquisition.md`](wasm_resource_acquisition.md) remains the host
protocol. `umber-vfs` owns the file portion of that protocol; JavaScript or a
native host still owns asynchronous I/O, URLs, authentication, caching, and
distribution selection.

## Goals

`umber-vfs` must:

- provide the same path, file, and transaction semantics in native and
  `wasm32-unknown-unknown` builds;
- let TeX and bibliography processing exchange `.bcf`, `.bbl`, `.aux`, and
  other generated files without filesystem access or subprocesses;
- retain user files, verified distribution resources, and generated files in
  distinct ownership layers;
- make accepted output atomic across a multi-stage, multi-pass build;
- report missing resources as deterministic typed batches;
- accept byte-identical duplicate provisioning idempotently and reject
  conflicting rebinding;
- use content identity for immutable bytes and cache keys;
- enforce per-file, per-layer, transaction, and aggregate limits before
  publication;
- preserve exact bytes, path spelling after canonicalization, and deterministic
  enumeration order; and
- permit cheap immutable snapshots and rollback without copying every file.

## Non-goals

`umber-vfs` does not:

- read the native filesystem, perform HTTP requests, derive URLs, or inspect
  environment variables;
- implement TeX input search, extension fallback, bibliography datasource
  search, `kpsewhich`, or another domain-specific lookup policy;
- interpret file contents;
- provide general POSIX filesystem behavior, permissions, symlinks, devices,
  memory mapping, or advisory locking;
- expose partially written streams to another concurrently executing stage;
- make application cache or eviction decisions; or
- replace `World` as the TeX engine's effect boundary.

## Crate boundary

The crate has no dependency on `umber`, the TeX execution crates, or the
bibliography crates. Higher layers adapt its exact-path and transactional APIs
to their own lookup rules.

```text
                       client resource policy
                                |
                         ResourceResponse
                                |
                           umber-vfs
                    /            |             \
             TeX resolver    bib resolver    output collector
                    \            |             /
                         project orchestrator
                                |
                        umber / umber-wasm
```

The crate may depend on `tex-content` for versioned, domain-separated content
identity. If that dependency would make naming misleading, the content-id
primitive should first move to an equally low-level neutral crate; the VFS
must not introduce a second hashing scheme.

## Namespace and canonical paths

The public namespace retains the implemented roots:

- `/job/...` contains user inputs and generated job files;
- `/texlive/...` contains verified distribution resources.

No other public root is initially valid. Layers are not encoded into path
strings: a user-supplied `/job/main.bbl` and a generated `/job/main.bbl` share
one logical path, with the build overlay controlling which generation is
visible.

`VirtualPath` canonicalization continues to:

- require `/` separators;
- remove empty and `.` components;
- reject `..`, NUL, backslash, colon, and URL-shaped syntax;
- reject paths that do not name a file;
- restrict absolute user paths to `/job`;
- restrict distribution paths to `/texlive`; and
- preserve Unicode path components as supplied rather than consulting the
  host filesystem's normalization or case rules.

Canonicalization is syntax, not search. Default extensions and search-area
ordering belong to the TeX or bibliography resolver that requested a file.

## File representation and identity

Every complete file is immutable:

```rust
pub struct VirtualFile {
    path: VirtualPath,
    bytes: Arc<[u8]>,
    content_id: ContentId,
    origin: FileOrigin,
}

pub enum FileOrigin {
    User,
    Resolved(ResourceRequestKey),
    Generated {
        producer: ProducerId,
        build: BuildId,
        stage: StageId,
    },
}
```

The content identity covers the exact bytes and a VFS file-content schema
domain. It excludes path, origin, registration order, allocation identity, and
build generation. A separate binding identity combines a canonical path with
its content identity when a cache must distinguish the same bytes at different
logical paths.

Files use `Arc<[u8]>` or an equivalent immutable owner. Read APIs return a
borrowed or shared immutable view; they never return a mutable byte buffer.

## Layers and lookup

A workspace owns four logical layers:

1. **User layer.** Explicit files supplied by the application. They are
   immutable across accepted incremental history except for the separately
   managed root editor buffer.
2. **Resolved-resource layer.** Files supplied in response to typed resource
   requests. Each request key is permanently bound to one verified object for
   the relevant session history.
3. **Accepted generated layer.** Files from the last complete accepted build.
4. **Pending generated layer.** Files produced by the build currently being
   attempted.

An executing pending build reads `/job` in this order:

```text
latest pending stage output
    -> earlier pending stage output
    -> accepted generated file when not invalidated by the build
    -> user file
```

Distribution reads resolve only through verified resources under `/texlive`.
Domain resolvers may search multiple exact candidates, but the VFS itself
never guesses a path.

The orchestrator must invalidate generated files whose producer is scheduled
to rerun. An invalidated old `.bbl`, for example, is not visible to the
bibliography stage that will replace it, while it may remain visible to an
initial TeX pass when the multipass policy intentionally uses the last
accepted bibliography result.

Directory enumeration is explicit, bounded, and lexically ordered by
canonical path. No semantic algorithm may depend on map iteration order.

The implemented storage handle owns an `Arc`-backed copy-on-write generation.
Capturing or cloning a snapshot therefore retains one generation without
copying its maps or file bytes. A later storage mutation copies only the
generation header and changed ownership layer; existing snapshots continue to
observe their exact earlier generation.

## Transactions and build atomicity

There are two transaction scopes:

- a **stage transaction** captures all files written by one TeX or
  bibliography invocation; and
- a **build transaction** contains the ordered successful stage commits for
  one requested document revision.

```rust
pub struct VirtualFs { /* accepted state */ }
pub struct BuildTransaction<'a> { /* private build overlay */ }
pub struct StageTransaction<'a> { /* one producer's write set */ }

impl VirtualFs {
    pub fn begin_build(&mut self, plan: BuildPlan) -> BuildTransaction<'_>;
}

impl BuildTransaction<'_> {
    pub fn snapshot(&self) -> VfsSnapshot;
    pub fn begin_stage(&mut self, producer: ProducerId) -> StageTransaction<'_>;
    pub fn accept(self) -> AcceptedBuild;
    pub fn discard(self);
}
```

A stage reads one snapshot. Its writes are private until the stage succeeds.
Committing the stage appends a new overlay generation visible to the next
stage in the same build. Failure or a missing-resource suspension discards the
stage write set; it does not expose truncated files.

Accepting the build atomically replaces the accepted generated layer and its
metadata. Discarding it leaves the previous accepted build, editor revision,
and outputs unchanged.

Within one stage, an output stream may be opened, appended, and closed using
the semantics required by `World`. The VFS stores no incomplete file after
the stage ends. Two producers writing the same path in one build require an
explicit replacement declared by the build plan; an undeclared collision is a
typed error.

## Reads, writes, and stage adapters

The base VFS API is byte-oriented and synchronous:

```rust
impl VfsSnapshot {
    pub fn get(&self, path: &VirtualPath)
        -> Result<Option<&VirtualFile>, SnapshotError>;
    pub fn contains(&self, path: &VirtualPath) -> Result<bool, SnapshotError>;
    pub fn list(&self, prefix: &VirtualPath, limit: usize)
        -> Result<Vec<VirtualPath>, SnapshotError>;
    pub fn list_root(&self, root: VirtualRoot, limit: usize)
        -> Result<Vec<VirtualPath>, SnapshotError>;
}

impl StageTransaction<'_> {
    pub fn write(&mut self, path: VirtualPath, bytes: Vec<u8>)
        -> Result<(), VfsError>;
    pub fn finish(self) -> Result<StageCommit, VfsError>;
    pub fn discard(self);
}
```

TeX continues to access bytes through `World` and its resolvers. An adapter
opens exact VFS files as `InputSource` values and publishes committed memory
outputs into a stage transaction. The bibliography layer uses its own adapter
over the same snapshot and transaction types. Neither engine receives mutable
access to the workspace maps.

`list` includes an exact prefix binding and descendants separated by a path
component boundary; it never treats a byte-prefix sibling as a descendant.
`list_root` covers the complete `/job` or `/texlive` namespace. Both merge the
ordered ownership layers directly, return each visible path once, and fail
before allocating more than the caller's result limit.

Snapshots may be created with an immutable set of invalidated accepted-output
paths. Such a path still resolves through pending output or the user layer when
present. Explicitly invalidating a snapshot makes every clone sharing its
validity token return `SnapshotError::Stale`; storage mutation itself does not
make retained snapshots stale. `SnapshotRetention` reports every binding and
logical file byte kept alive by the retained generation, including hidden
shadowed bindings.

## Resource requests and registration

A resource request identifies what an engine needs without prescribing where
the host obtains it:

```rust
pub struct FileRequestKey {
    pub domain: ResourceDomain,
    pub kind: FileKind,
    pub normalized_name: String,
}

pub struct ResolvedFile {
    pub request: FileRequestKey,
    pub virtual_path: String,
    pub bytes: Vec<u8>,
    pub expected_digest: Option<FileContentId>,
}
```

The vocabulary initially covers TeX inputs, TFM files, format images,
bibliography control files, bibliography data, bibliography configuration,
XML/schema data, and explicitly typed generic assets. The domain and kind are
part of identity even when two requests share a normalized name.

The implemented `ResourceDomain` values are TeX, bibliography, and generic.
Wire names are defined by these Rust values and reused by `umber-wasm`; the
binding does not maintain a second kind table.

Registration validates:

- that the response repeats an outstanding request key;
- canonical path and permitted root;
- declared and hard byte limits;
- an expected digest when present;
- type-specific validation delegated to the requesting subsystem; and
- absence of a conflicting prior binding.

The VFS performs generic validation and stores the object only after the
requesting subsystem accepts its structure. Re-registering the same request,
path, and bytes is a no-op. Any different binding is a typed conflict.

`FileRequestBatch` stores outstanding requests in sorted sets, deduplicates by
the complete domain/kind/name key, and lets a required request dominate the
same prefetch hint. `FileProvisioner` accepts partial and permuted responses
atomically, retains identical duplicate registrations as no-ops, and exposes
typed unexpected-request, kind, path, digest, conflict, path-conflict, limit,
and no-progress failures. The combined compile session retains the existing
`NeedResources` required-versus-hint model around this file-only boundary.

## Root editor buffer

The mutable editor root remains owned by the persistent compile-session
machinery because its piece layout, revisions, and source-coordinate mapping
belong to `tex-incr`. The VFS exposes the accepted root revision as a synthetic
immutable `/job/...` file in each snapshot.

Applying a source patch creates a candidate root file for the pending build.
It does not mutate the accepted user layer. Accepting the compile revision
publishes the new root identity together with the generated build; rollback
retains the prior root and generated files.

## Limits and accounting

File-related fields currently embedded in `umber::SessionLimits` move into a
composable `VfsLimits` value:

```rust
pub struct VfsLimits {
    pub user_files: usize,
    pub resolved_files: usize,
    pub one_file_bytes: usize,
    pub user_bytes: usize,
    pub resolved_bytes: usize,
}
```

Generated-file count and byte limits join this value when stage and build
transactions are implemented. Today `VirtualCompileSession::SessionLimits`
preserves its public compatibility fields but delegates file hard-ceiling,
replacement, and provisioning checks to `VfsLimits` and `FileProvisioner`.

Limits use checked arithmetic and are enforced before allocation where the
declared size is known, during bounded stream growth, at stage commit, and at
build acceptance. Replaced generations cease to count once no live snapshot
or accepted history retains them. Telemetry reports logical bytes separately
from retained shared allocations.

The implemented snapshot accounting exposes retained-generation binding and
logical-byte totals. Transaction and session owners will aggregate those
values with allocation-level telemetry when they adopt snapshots.

No subsystem can bypass VFS accounting by returning an auxiliary output in a
separate unbounded collection.

## Determinism and WebAssembly

The crate uses no filesystem calls, environment variables, locale queries,
networking, subprocesses, native threads, native XML libraries, or platform
path comparison. It must compile under `wasm32-unknown-unknown` in the ordinary
workspace browser gate.

All observable ordering is explicit. Hash maps may be used as private lookup
accelerators only when their iteration order cannot affect requests,
diagnostics, output enumeration, or cache identity.

The JavaScript binding is a representation adapter. It transfers typed
requests and byte responses and may drive an asynchronous resolver loop, but
it does not implement alternative path or layer semantics.

## Errors

Typed failures include:

- invalid or out-of-root path;
- missing exact file;
- unrecognized or mismatched request response;
- content digest mismatch;
- conflicting immutable registration;
- undeclared generated-path collision;
- write after stream or transaction closure;
- per-file or aggregate limit violation;
- resource retry without progress;
- use of a snapshot after its owning stage is invalidated; and
- accepting a build against a stale root revision.

Errors may expose canonical virtual paths, request keys, content identities,
and limits. They do not expose host URLs or treat file bytes as trusted markup.

## Integration with project compilation

The project orchestrator uses one build transaction for an entire converging
LaTeX job:

```text
pending root and last accepted generated inputs
    -> TeX stage writes .aux/.bcf/etc.
    -> bibliography stage reads .bcf and data, writes .bbl/.blg
    -> TeX stage reads .bbl and writes the next auxiliary generation
    -> repeat until the selected convergence set is stable
    -> atomically accept root revision, generated files, and rendered output
```

A resource miss at any stage returns the accumulated deterministic request
batch. Provisioned immutable resources are retained, while the failed stage's
writes are discarded. The orchestrator may restart from a retained safe stage
boundary or rerun the build; both paths must produce identical accepted files.

## Testing

Crate-internal tests cover canonicalization, layer precedence, immutable
registration, transaction visibility, collision policy, limits, snapshots,
rollback, and deterministic enumeration. Public-boundary tests use one
integration-test binary and exercise native and WASM representation adapters.

Required property tests include:

- arbitrary invalid paths never escape the two public roots;
- response permutation and chunking do not alter the accepted workspace;
- a discarded stage or build cannot change accepted reads;
- accepting then snapshotting is equivalent to constructing the same layers
  directly;
- content identity is independent of registration and allocation order;
- identical duplicate registration is idempotent and every conflict fails;
- limit accounting never wraps; and
- native and WASM request/result encodings round-trip to the same Rust values.

The end-to-end gate compares cold and retained multi-pass builds and requires
byte-identical generated files and DVI.

## Migration plan

1. **Complete.** Add `umber-vfs` with the current `VirtualPath` behavior and
   exhaustive parity tests. `umber` consumes and re-exports this public path
   API; TeX request-name and extension policy remains in the driver.
2. **Complete in `umber-vfs`.** Add domain-separated immutable file and path
   binding identity, shared byte ownership, provenance, and deterministic
   user, resolved-resource, accepted-generated, and pending-generated storage.
3. **Complete.** Move domain-qualified file request keys, deterministic file
   batches, resolved-file validation/provisioning, request-bound origins, and
   file-related limit checks into `umber-vfs`. `VirtualCompileSession` consumes
   this registry and re-exports the shared value types.
4. **Complete in `umber-vfs`.** Add cheap retained snapshots, exact lookup,
   accepted-output invalidation, stale-clone protection, and bounded lexical
   enumeration.
5. Add build transactions over the accepted and pending generated layers;
   migrate persistent compile-session atomicity to them.
6. Adapt the existing TeX resolver and memory-output collector to VFS
   snapshots and stage transactions without changing public compile behavior.
7. Add the bibliography resource kinds and adapters defined in
   [`bib.md`](bib.md).
8. Implement native multi-stage orchestration, then expose the identical state
   machine through `umber-wasm`.
9. Remove superseded private file maps and duplicate path/request types after
   all native, incremental, and browser tests use `umber-vfs`.

## Exit criteria

The design is complete when:

- TeX and bibliography processing exchange generated files through one VFS
  without native filesystem access or subprocesses;
- native and WASM builds run the same Rust stages and observe identical paths,
  requests, diagnostics, and bytes;
- missing files from either engine participate in one batched resource loop;
- a failure or resource miss at any stage leaves the previous accepted build
  unchanged;
- generated-file convergence and invalidation are explicit and tested;
- duplicate provisioning, collision, limit, cancellation, and no-progress
  behavior are typed and deterministic;
- persistent revision retention charges every live VFS allocation; and
- no legacy private virtual-file store remains in `umber` or `umber-wasm`.
