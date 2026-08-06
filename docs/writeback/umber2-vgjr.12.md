# umber2-vgjr.12 — fixture contract and catalogue compaction closeout

The exact combined implementation tree is
`1aceb8ae8e5bd6b3f8fbf2a9a3a8fd2b961d128a`. All four children are closed with
substantive writebacks. `test-support::closed_case::FixtureCase` is the single
typed contract and Git-inventory consumer boundary; test-support validates,
serializes, and stages candidates without publishing; fixturegen's cohort
transaction is the sole atomic mutation and publication owner. Command-semantic
V2 alone owns inferred identity, capture, routes, channels, statuses, xfails,
and generated schema. The TeX82 catalogue uses one typed deferred default plus
sorted explicit overrides. `corpus-manifest` still has zero dependencies.

The migration deleted the detached 173-line capture list, 467-line duplicate
census, 10,448-line V1 command-semantic manifests, 11,047-line TeX82
disposition catalogue and its generator path, plus repeated validators,
inventory serializers, and direct fixture mutation paths. Digest and census
proof preserves all 203 command-semantic cases, 1,233 explicit expected
strings, capture selection, statuses, xfails, channels, and the exact TeX82
resolved map with 946 reviewed / 434 deferred modules and 106 covered / 45 gap
properties. Negative gates cover path normalization, traversal, missing,
extra, reordered, hash, closure, schema, rollback, retry, and partial
publication failures; local hash-optional fixture edits remain valid.

Across commits `5675a359f`, `fbcaa9616`, `0634d2800`, `5a012b9e7`, and
`1aceb8ae8`, authored source/configuration is +1,676/-1,173 (net +503), and
documentation is +136/-33 (net +103). Declarative fixtures are +6,834/-21,244
(net -14,410); generated schema evidence is +443/-199 (net +244); generated
lockfiles are +333/-8 (net +325). Total tracked change is +9,422/-22,657, or
13,235 lines of net deletion, with no binary fixture changes. The authored
500-900-line reduction forecast is not met and is durably tracked by
`umber2-vgjr.23`; repetitive fixture deletion is not credited as authored
reduction.

The focused consumer selection and all 33 fixturegen tests passed after
uncapped `--no-run` builds and under `MemoryMax=512M`. The complete native
workspace suite passed after its uncapped `--no-run` build and under
`MemoryMax=1G`. `CARGO_BUILD_JOBS=1 scripts/check.sh` passed all four gates
under `MemoryMax=1G`. Tests and gates were serialized with finite timeouts.
