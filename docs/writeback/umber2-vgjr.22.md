# umber2-vgjr.22 -- resource-lifecycle forecast reconciliation

## Owner decision

The portfolio owner accepts Program 3 at its measured 668 lines of net authored
Rust deletion and retires the original 1,200--1,900-line reduction forecast. No
unimplemented deletion remains scheduled, and no moved, generated, historical,
fixture, documentation, binary, or compatibility-gated lines are credited.

The estimate correctly identified duplicate acquisition loops, admission maps,
and non-driving Rust resource planes, but treated much of their gross deletion
as net reduction. The replacement necessarily introduced one generic ordered
admission state machine, one policy-parameterized downloader, one locked store
entry transition, typed bindings for domain validation, and independent proofs
for the preserved failure paths. Deleting those replacements to reach the
forecast would restore implicit or duplicated authority rather than simplify
the implemented lifecycle.

## Authority and caller audit

- `umber_vfs::ResourceLifecycle` is the only mutable admission-transition
  authority. `ProjectWorkspace`, `VirtualCompileSession`, and
  `LatexProjectSession` specialize it for file, OpenType, and PK keys. Their
  retained parsed-font maps, VFS paths, request descriptors, and batch frontier
  sets are domain values or bounded execution projections; none can authorize a
  positive or negative transition independently.
- `umber_fetch::VerifiedDownloader` is the only native transport, length,
  digest, retry, and cooperative-cancellation loop. Object and manifest fronts
  select different policies and translate public errors without downloading or
  verifying bytes themselves. `FetchClient` coordinates bounded ordered
  batches, while `DistributionClient` owns native source selection and binds
  the downloader to a store; neither duplicates the downloader.
- `BlobStore::resolve_entry` is the only locked validation, quarantine,
  compatibility migration, construction, and no-clobber publication
  transition. Object, manifest, generated-format, and public compatibility
  methods construct a `VerifiedBlobSpec` and enter that transition. Anchored
  Unix filesystem operations remain the platform implementation beneath it,
  not another state machine.
- `umber-distribution` owns authenticated catalogue parsing and selection.
  Native `umber` owns local/cache/remote/offline ordering and blocking batch
  scheduling. Authored JavaScript owns asynchronous provider precedence,
  browser fetch/cache, abort, and worker delivery. These distinct host policies
  converge only at the shared typed request/response admission boundary.
- `tex-exec` owns suspended engine needs, `bib-engine` owns detached
  bibliography closure, and `tex-incr` owns candidate acceptance. Their
  resource views cannot admit transport bytes or publish another phase's
  candidate. Cancellation and rollback discard only their private work while
  immutable admitted session resources survive.

Repository-wide caller inspection found no surviving Rust
`OutputResourcePlan` or Rust `CompositeResourceResolver`, no second download or
store-entry loop, and no parallel positive/negative admission registry. The
authored JavaScript `CompositeResourceResolver` remains a live browser
scheduler used by worker composition. The public `FetchClient`, manifest fetch,
and `BlobStore` methods are compatibility facades over the named internal
authorities; retiring them requires a separate API decision and would not
remove their underlying behavior.

## Proof audit

The retained tests are behavior owners, not shadow implementations.
`umber-vfs` covers ordering, intent promotion, partial and permuted batches,
idempotence, conflicts, atomic admission, hints, probes, progress, and
rollback. `umber-fetch` covers cache reuse, offline hits, manifest policy,
corruption, exact and bounded lengths, retries, cancellation publication,
bounded batching, compatibility migration, and concurrent no-clobber
publication. `umber` covers native precedence, retained retries, atomic font
and file batches, cache-only late bytes after cancellation, bibliography pass
isolation, and revision rollback. The Rust WASM and authored JavaScript suites
cover typed delivery, provider-scoped absence, asynchronous cancellation,
verified browser caching, worker composition, and PK identity.

Compacting these proofs would not delete a production authority and would
weaken the acceptance evidence required by the program. Likewise, merging the
native and browser schedulers would cross the contract's host boundary, and
merging domain objects into `ResourceLifecycle` would create the prohibited
giant resolver.

## Exact accounting

Independent `git show --numstat` classification of implementation commits
`fbdaaa997`, `cb8cd4a96`, `c576d4fb9`, `8e5af7538`, `3923f4167`, and
`d5f896aa8` reproduces the closeout totals.

| Category                  | Additions | Deletions |  Net |
| ------------------------- | --------: | --------: | ---: |
| Production Rust           |     1,005 |     1,307 | -302 |
| Authored Rust proof tests |        74 |       440 | -366 |
| All authored Rust         |     1,079 |     1,747 | -668 |

Declarative configuration adds four lines. Documentation and repository maps
add 421 and delete 43 lines. The linked missing-review repair adds 561
documentation lines. Those categories remain outside authored-Rust reduction,
and generated and binary changes are zero. The 532-line difference from the
forecast floor is retired rather than converted into unsupported credit or a
line-count-driven deletion target.

## Verification

The coordinator's uncapped six-job `cargo test -q --tests --no-run` build
passed before this audit. Focused lifecycle, fetch, distribution, and native
Umber execution passed 537 tests under `MemoryMax=512M`; all 91 authored
JavaScript tests passed under the same cap. The real wasm-bindgen Node gate
passed under `MemoryMax=1G`: all three wire tests and both typed virtual-font
acquisition tests passed, while the explicitly browser-only suite was skipped
by the Node runner as designed.

The complete native `cargo test -q --tests` routine suite passed under
`MemoryMax=1G`. The exact documentation tree then passed all four
`scripts/check.sh` gates under the same hard cap with six Cargo jobs.
