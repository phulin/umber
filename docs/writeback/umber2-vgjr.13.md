# umber2-vgjr.13 — WASM wire, catalogue, and session closeout

Implementation tree: `bd75c2cc1c476c25c0d30535d57518266c325f4f`.

## Surviving authorities and deleted predecessors

`umber-wasm::wire` schema 1 is the sole structural authority for public
options, requests, responses, attempts, outputs, diagnostics, metrics,
observations, rendered-source values, catalogue plans, and stable error codes.
Binding adapters perform one conversion to or from private engine values.
`serde-wasm-bindgen` and `serde_bytes` preserve `Uint8Array`, omitted optional
properties, and safe-integer validation. The checked-in TypeScript custom
section is generated from the DTOs and checked byte-for-byte. The handwritten
declaration block and manual result/resource `Object` and `Reflect` conversion
tables are deleted.

`SessionDriver` is the one direct and worker-realm retry, resource-delivery,
progress, cancellation, and disposal core. `WorkerRpcClient` is the one
request-correlation, timeout, owner-abort, progress, message-error, transfer,
and teardown core. Public direct, retained-editor, one-shot-worker, and
retained-worker facades remain package adapters. Their duplicate orchestration,
binding preparation, resolver composition, and format-selection branches are
deleted.

`umber-distribution` is the sole strict root, shard, named-format, partition,
canonical-byte, authentication, duplicate-rejection, selection, and
required-before-hint plan authority. `texlive-wasm-publish::PreparedPublication`
is the sole full/HTML staging and readback path. JavaScript retains HTTP,
persistent cache, concurrency, cancellation, budgets, public resource-key
adaptation, and response materialization. Its former catalogue JSON scanner,
partition hashing, record parsers, and selection walk are deleted, as are the
publisher shadow DTO/canonicalizer, duplicated staging paths, and executable
HTML MVP catalogue.

## Compatibility decisions

The schema-1 monolithic `Manifest` reader and record model remain because the
documented `texlive-wasm-publish --shard-existing` offline conversion consumes
them and prepared publication constructs the same input model. The dead public
monolithic selection planner, pretty writer, writer-only helpers, and stale
selection fixture are deleted. The publisher filesystem/readback adapter,
native single-shard adapter, three typed WASM catalogue exports, and authored
JavaScript request/response adapters retain live callers and distinct host or
package policy; none is a second catalogue authority.

The JavaScript resolver's `JSON.parse` consumes the root text already strictly
validated and canonically serialized by Rust only to obtain transport fields.
It does not authenticate, partition, reject catalogue duplicates, select
records, or define request order. Incremental HTML projection remains the
separately documented receiver boundary and is not a manual mirror of the
wire families migrated here.

## Exact authored accounting

| Child                                                 | Authored additions | Authored deletions |      Net |
| ----------------------------------------------------- | -----------------: | -----------------: | -------: |
| `.13.1` wire DTO authority                            |              1,204 |                  7 |   +1,197 |
| `.13.2` session/worker drivers and closeout proof     |                490 |                471 |      +19 |
| `.13.3` catalogue, publication, and provenance repair |                729 |              1,070 |     -341 |
| `.13.4` binding migration                             |              1,378 |              1,684 |     -306 |
| `.13.5` catalogue compatibility audit                 |                 17 |                369 |     -352 |
| `.13.6` wasm-bindgen proof migration                  |                 62 |                 32 |      +30 |
| **Program 13 total**                                  |          **3,880** |          **3,633** | **+247** |

These figures count authored Rust, JavaScript, and proof tests. They exclude
the generated 67-line TypeScript declaration, 13 deleted declarative
fixture/inventory lines, documentation and guidance, moved code, retained
compatibility surfaces, committed catalogue records, lockfiles, and binary
assets. Unlike the earlier implementation-only receipts, the final `.13.2`
row includes the 12-addition/9-deletion direct/worker diagnostic proof and the
final `.13.3` row includes the 8-addition/3-deletion catalogue provenance
repair. The catalogue-only `.13.3` plus `.13.5` total is +746/-1,439, net
-693; its retired 800--1,100 forecast therefore overstated deletion by
107--407 lines. No unimplemented deletion is carried forward.

## Verification

Fresh closeout verification used `CARGO_BUILD_JOBS=6` and finite timeouts.
Full native, focused publisher, and wasm32 test builds, the optimized package,
and the native browser binary compiled uncapped. Under `MemoryMax=512M`, all 17
`umber-distribution` tests, 17 publisher tests, 21 native catalogue/offline
tests, three native `umber-wasm` tests, and 89 authored Node tests passed.

Under `MemoryMax=1G`, real `wasm-pack test --node crates/umber-wasm` compiled
the complete wasm-bindgen harness and passed the schema golden plus both
virtual-font acquisition tests; the browser-only suite compiled and was
correctly skipped by the Node runner. The optimized package passed its
TeX--bibliography--TeX Node lifecycle and `npm pack --dry-run` reported exactly
36 files. The complete native `cargo test -q --tests` suite passed. The final
uncapped six-job `scripts/check.sh` run passed all four gates.

The browser runner reached its explicit prerequisite check and stopped with
`ENOENT` for `/usr/bin/google-chrome`. Firefox, Firefox ESR, GeckoDriver,
Google Chrome, Chrome stable, Chromium, Chromium Browser, and ChromeDriver were
absent from `PATH`; browser execution is therefore unavailable and is not
reported as passing. Open environment/proof follow-ups `umber2-5zie` and
`umber2-3slp` retain their existing scopes.

Publisher execution refreshes its standalone lock with dependency reachability
already tracked by `umber2-ss53`; that unrelated generated change was restored
after the passing 17-test run. It is not credited to this program and does not
represent a catalogue, package, or runtime failure.
