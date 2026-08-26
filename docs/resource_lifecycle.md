# Canonical resource identity and lifecycle

Status: normative contract for Beads epic `umber2-vgjr.3`. The shared ordered
admission state machine and file, OpenType-font, and PK-font production callers
are implemented. Domain validation remains at the typed compile-session
boundary. Existing distribution, native, and WebAssembly identities are
compatibility inputs. This contract does not create a second resolver or
scheduler.

Repository-owned distribution and font/resource identities are deterministic
64-bit aHash v1 values with explicit domains and fixed seeds. They provide
stable selection and accidental-corruption detection across native and WASM,
not adversarial authenticity. Persisted format schema 12, page-artifact schema
24, font response schema 3, distribution roots 6/7, shards 3/4, and cache
envelope schema 2 reject their SHA-era predecessors. External source locks,
oracle fixtures, corpus evidence, and parity output digests remain SHA-256
because those external/evidentiary contracts are outside resource admission.

This document defines the typed lifecycle connecting engine resource needs,
VFS admission, verified acquisition, incremental candidates, bibliography
closure, native hosts, and browser hosts. It complements the implemented
contracts in [umber_vfs.md](umber_vfs.md),
[wasm_resource_acquisition.md](wasm_resource_acquisition.md),
[ctan_resource_fetch.md](ctan_resource_fetch.md), and
[distribution_manifest.md](distribution_manifest.md).

> One semantic request key survives unchanged from the requesting subsystem to
> an immutable positive or negative session binding. Catalogue lookup, source
> selection, acquisition, caching, validation, and revision publication are
> typed transitions around that key, never replacements for it.

## Boundary rules

The lifecycle must preserve exact identity, ordering, and wire encodings;
distinguish required reads, blocking probes, and optional hints; distinguish a
provider miss from authoritative absence; verify and domain-validate bytes
before visibility; and retain suspended candidates without publishing their
resources, effects, generated files, or outputs.

Host I/O, retries, clocks, cancellation handles, URLs, cache paths, futures,
threads, and JavaScript objects never enter engine or incremental snapshots.
Native blocking work and browser asynchronous work remain separate adapters.
TeX search, bibliography search, font/image/format parsing, and output closure
remain with their domain owners. A catalogue key, object digest, VFS path,
URL, and cache key are not semantic request identities.

## Canonical vocabulary

Names below describe target semantic roles. Migration may preserve current
public names through boundary adapters.

```rust
pub enum ResourceKey {
    File(FileRequestKey),
    OpenTypeFont(FontRequestKey),
    PkFont(PdfPkFontRequest),
    LegacyFontMapping(LegacyMappingRequestKey),
}

pub enum RequestIntent {
    Required,
    Probe,
    PrefetchHint,
}

pub struct ResourceRequest {
    pub key: ResourceKey,
    pub original_name: Option<String>,
}
```

`ResourceKey` alone defines equality, ordering, deduplication, response
matching, immutable binding, and retry progress. `original_name` is bounded
search and diagnostic context, not identity. `RequestIntent` is policy, not
identity. For the same key, `Required` dominates `Probe`, and either blocking
intent dominates `PrefetchHint`. Requests use typed total order; response
arrival order cannot affect publication or diagnostics.

The existing semantic identities are frozen compatibility inputs:

| Variant             | Complete identity                                                                                             | Current constructor authority |
| ------------------- | ------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| File                | `ResourceDomain`, `FileKind`, normalized relative name                                                        | `umber-vfs`                   |
| OpenType font       | logical name, face, variation instance and coordinates, feature policy, purposes, direction, script, language | `tex-fonts`                   |
| PK font             | exact TeX name bytes, resolved DPI, frozen mode                                                               | `tex-fonts`                   |
| Legacy font mapping | schema, TFM aHash64, layout-policy version, purpose, optional encoding catalogue                              | `umber-distribution`          |

The target owner of the closed key union, intent, batching, generic admission
state, and binding metadata is `umber-vfs`. Domain crates still construct and
validate their key fields. No generic string-key constructor may bypass them.
File keys retain all current domain and kind wire names. Their syntax-normalized
relative name is neither a VFS path nor acquisition policy.

### Transport adapters

`umber-distribution` exclusively owns catalogue-key encoding, parsing, and
selection. A transport key selects metadata but never replaces the semantic
key on a request or response.

| Semantic request                       | Existing canonical catalogue key                                                 |
| -------------------------------------- | -------------------------------------------------------------------------------- |
| TeX input                              | `tex:<normalized-name>`                                                          |
| TFM                                    | `tfm:<normalized-name>`                                                          |
| Classic AUX                            | `bib-aux:<normalized-name>`                                                      |
| Classic BibTeX datasource              | `classic-bib:<normalized-name>`                                                  |
| Classic BibTeX style                   | `bst:<normalized-name>`                                                          |
| VF, PDF map, encoding, or font program | `tex:<normalized-name>`                                                          |
| OpenType font                          | existing canonical `font:1:...` encoding                                         |
| Legacy mapping                         | existing canonical `legacy-mapping:1:...` encoding                               |
| PK font                                | no implicit catalogue key; an explicit provider maps the complete typed identity |

PDF semantic file kinds deliberately share the TeX catalogue namespace. A
positive or negative response repeats the original semantic kind; the
`tex:<name>` selection key is discarded. Other file kinds have no implicit
hosted mapping.

One-to-one adapters must satisfy `decode(encode(key)) == key` and reject
noncanonical encodings. A many-to-one adapter returns a selection token that
contains both original semantic key and transport key, and admission matches
only the former. Migration tests reuse current Rust, JavaScript, manifest,
font, and PK fixtures and require byte-exact encoding equality.

## Admission state

`umber-vfs` owns one bounded ledger entry per semantic key:

```rust
pub enum AdmissionState {
    Unseen,
    Outstanding(RequestIntent),
    Admitted(AdmittedResource),
    Unavailable(AbsenceEvidence),
}
```

`Unseen` is absence from the ledger. `Outstanding` is candidate-local response
authorization, not a published binding. `Admitted` and `Unavailable` are
immutable session-history bindings that survive retries and revisions and
cannot transition into each other.

`umber_vfs::ResourceLifecycle` is the shared implementation of these
transitions. `ProjectWorkspace` specializes it with `FileRequestKey` and
canonical resolved paths. Compile and project sessions specialize it with the
typed OpenType and PK identities plus their validated receipts. Direct
compilation, retained retries, multipass TeX and bibliography closure, and
WebAssembly-delivered responses therefore use the same transition authority.
Starting a pass or retry batch cancels only the previous outstanding
authorizations; immutable bindings remain available to later passes.

| From        | Event                                                   | To          |
| ----------- | ------------------------------------------------------- | ----------- |
| Unseen      | issue request                                           | Outstanding |
| Outstanding | stronger duplicate intent                               | Outstanding |
| Outstanding | verified and domain-valid positive response             | Admitted    |
| Outstanding | authoritative negative for required or probe            | Unavailable |
| Outstanding | hint omitted, failed, cancelled, or negatively answered | Unseen      |
| Admitted    | identical response and metadata                         | Admitted    |
| Unavailable | identical authoritative negative                        | Unavailable |

All other transitions are typed conflicts. A provider miss does not create
`Unavailable`; a hint never creates a negative binding; cancellation and
failure create no binding. Included partial responses commit atomically after
all validate. An empty or non-progressing blocking batch is a typed retry
failure.

An admitted binding retains semantic key, immutable bytes, content identity,
generic verification receipt, domain-validation receipt, and, for files, the
canonical VFS path. Font and PK bindings additionally retain their canonical
program or instance identity. Nothing is visible to VFS or engine lookup
before `Admitted`.

## Resolution and verification

Provider composition and admission use different vocabularies:

```rust
pub enum ProviderOutcome {
    Candidate(AcquiredCandidate),
    Miss { key: ResourceKey, provider: ProviderId },
    Failure(AcquisitionFailure),
    Cancelled,
}

pub enum ResolutionOutcome {
    Verified(VerifiedCandidate),
    AuthoritativeUnavailable(AbsenceEvidence),
    Failure(ResolutionFailure),
    Cancelled,
}
```

A `Miss` is scoped to one provider. Ordered composition may try the next
source. Only the scheduling policy owner, after all applicable providers miss,
may create `AuthoritativeUnavailable`. Corruption, invalid catalogues,
validation, limits, I/O failure, and cancellation are not misses.

`AcquiredCandidate` is private to the scheduler and may contain source details.
`VerifiedCandidate` is host-neutral and contains no handle. Domain validation
consumes it and returns an admission-ready value or failure; it never publishes
partially validated bytes.

```rust
pub struct VerificationSpec {
    pub length: LengthExpectation,
    pub digest: Option<ExpectedDigest>,
}

pub enum LengthExpectation {
    Exact(u64),
    AtMost(u64),
}

pub struct VerificationReceipt {
    pub actual_bytes: u64,
    pub content_id: ContentId,
    pub ahash64: Option<[u8; 8]>,
    pub source: SelectionProvenance,
}
```

Catalogue objects require exact length and aHash64. Local objects without a
declared digest remain bounded by `AtMost`; admission still computes their
domain-separated content identity. Declared VFS content, font object/program,
or PK identities are additional checks. An omitted declaration never skips
hard limits or domain validation.

Provenance contains a bounded typed source class and verified catalogue
identity when applicable, but no credential, URL, native path, JavaScript
object, cache timestamp, or retry history. Cache hit versus download is
telemetry and cannot change admission or accepted output.

`umber-fetch` owns native verification, quarantine, download, and atomic
store entry. Browser JavaScript owns equivalent transport verification before
crossing into WASM. Rust admission rechecks semantic declarations and domain
structure; an adapter never bypasses it.

The native implementation has exactly two internal transition authorities.
One policy-parameterized downloader enforces HTTPS or loopback transport,
exact or bounded length, aHash64, retries, and cooperative cancellation for
both objects and manifests. One per-key locked store-entry state machine owns
current-entry verification, semantic quarantine, compatibility migration,
construction, durable no-clobber publication, and winner revalidation for
objects, manifests, and generated formats. Public compatibility façades select
policy and translate errors; they do not repeat either workflow.

## Publication transitions

There are three different publication boundaries:

1. **Verified object.** `umber-fetch::DistributionClient` may atomically write
   a native object to `BlobStore`; browser JavaScript may write to an
   application cache. This enables transport reuse but does not admit bytes.
2. **Session admission.** `umber-vfs` atomically installs an immutable positive
   or authoritative-negative binding after matching, generic verification,
   and domain validation. Admission may survive revision failure because it is
   session input, not revision output.
3. **Revision acceptance.** `tex-incr` owns a private candidate until engine
   completion, bibliography and output-resource closure, VFS generated
   transaction completion, effect validation, and output construction all
   succeed. It then publishes root revision, generated generation, accepted
   input observations, effects, artifacts, and output together.

Engine suspension retains only semantic needs, retained execution state,
immutable admitted capabilities, and bounded candidate state. It retains no
resolver, downloader, cache transaction, native path, future, callback,
worker message, or URL. Rollback or cancellation drops outstanding
authorizations and private outputs, while immutable admitted session resources
remain.

`bib-engine` owns bibliography closure and detached generated files over one
immutable VFS snapshot. It performs no host I/O. `umber` owns the multipass
candidate transaction. Bibliography suspension publishes neither generated
files nor accepted input observations.

## Phase ownership

| Phase                                                 | Sole authority                                        |
| ----------------------------------------------------- | ----------------------------------------------------- |
| Semantic key union, intent, and admission             | `umber-vfs`                                           |
| Domain key construction and validation                | requesting engine, `tex-fonts`, or bibliography crate |
| Catalogue encoding, parsing, selection, verified miss | `umber-distribution`                                  |
| Native acquisition, verification, persistent store    | `umber-fetch::DistributionClient`                     |
| Suspension and retained resource capability           | `tex-exec`                                            |
| Candidate lifetime and revision acceptance            | `tex-incr`                                            |
| Native provider order and scheduling                  | `umber`                                               |
| Browser async provider order and scheduling           | authored JavaScript in `umber-wasm`                   |
| Bibliography closure and detached result              | `bib-engine`                                          |

No owner reconstructs another owner's index. Native and browser adapters
consume the same requests and return the same admission DTOs, but remain
distinct implementations: native may block and use anchored filesystem
persistence; browser code uses promises, fetch, application caches, abort
signals, and worker transfers.

## End-to-end transition

```text
domain need
  -> semantic key + intent
  -> candidate-local authorization
  -> native scheduler OR browser scheduler
  -> provider selection
       -> miss: continue
       -> failure/cancel: stop without binding
       -> acquired candidate
  -> generic verification
  -> domain validation
  -> atomic admission
  -> resume retained candidate
  -> complete engine/bibliography/output closure
  -> atomic revision acceptance
```

For a blocking key, all providers missing takes the alternative path through
authoritative negative admission. For a hint, misses, cancellation, or failure
end without binding or retry progress.

## Migration gates

Follow-on work must prove that every current file tuple round-trips exactly;
distribution file, font, and mapping strings remain byte-identical; PDF kinds
survive their many-to-one catalogue mapping; and PK name bytes, DPI, and mode
survive native and WASM DTOs. Sorting, intent promotion, partial batches,
duplicate idempotence, conflicts, provider miss, authoritative absence,
corruption, cancellation, and no-progress remain distinct.

Snapshot audits must find no host scheduling or I/O state. Separate native
local/cache/remote/offline and browser async/cache/worker tests run over common
lifecycle fixtures. Suspension, failure, cancellation, and stale candidates
must publish no generated files, effects, observations, artifacts, or output.

Migration is complete only when production callers use this contract and the
obsolete request unions, response maps, downloader fronts, and admission
registries are deleted. Temporary public compatibility adapters translate at
the boundary and cannot become a second authority.

## Rust resource-plane API decision

The exported Rust `OutputResourcePlan` and `CompositeResourceResolver` families
were retired under `umber2-vgjr.3.4`. Repository-wide caller audit found no
production Rust consumer: the plan only mirrored requests already selected by
the live session, and the resolver was only exercised by its own tests. Keeping
either API, including as a deprecated adapter, would retain a second
non-driving owner of resource state. The public Rust boundary is therefore the
ordered, bounded `NeedResources`/`ResourceResponse` protocol alone. Authored
JavaScript may retain an application-side composite resolver because it
actually drives that protocol; it is not engine state or a Rust compatibility
surface.

The removed planner's only live guard, the deduplicated request-union ceiling,
remains immediately before every session suspension. Required, probe, and
prefetch vectors retain their existing construction and order; the guard does
not rebuild them.

Exact issue diff accounting is 55 additions and 1,171 deletions in authored
Rust, a net deletion of 1,116 lines. Production Rust accounts for 28 additions
and 737 deletions (709 net); proof tests account for 27 additions and 434
deletions (407 net). Documentation and repository maps account for 40
additions and 26 deletions, so the complete tracked change is 95 additions and
1,197 deletions, or 1,102 lines of net deletion. No generated or binary assets
changed.
