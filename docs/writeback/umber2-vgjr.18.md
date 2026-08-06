# Tooling retirement program closeout

Issue: `umber2-vgjr.18`

## Authority and retirement audit

The final tree has one reference-process implementation:
`fixturegen::reference` owns executable lookup, deterministic environment,
staging, TeX/TFtoPL flags, output and status capture, and manifest-hash
verification. `fixturegen --reference-dvi` calls it directly and publishes
through `fixture_transaction.rs`. `refexec` and parity's feature-enabled live
API/CLI delegate to that kernel and remain supported compatibility/composition
surfaces. Generic DVI equality remains in `test-support`, semantic comparison
remains in `tex-command-stream`, and parity retains Umber execution and bounded
triage.

The only retired compatibility commands are the completed
`fixturegen --migrate-layout` and `--migrate-pdf-layout` modes. Their final
150-case and 15-case current-tree report identities remain in the
[`18.3` receipt](umber2-vgjr.18.3.md); ordinary regeneration, PDF/corpus/reference
publication, cohort sealing, rollback, retained-root recovery, and garbage
collection all still use the shared transaction owner. The only retired
historical surfaces are `dvi_page_snapshot`, `mode_list_rollback`, and the PNG
import prototype. Their adopted DVI, production journal, mutation-boundary,
semantic, bounded decoder, PDF, and corpus authorities remain.

Every other inventory row survives: all seven Plain TeX workloads, all six
edit-restart pairs and generated long case, command allocation/source identity,
layout/width/allocation, shipout, pure-memo edit, state/snapshot/cache/dependency
workloads, full-document and canonical oracle traces, public
`TraceOperation`/`TraceSummary`, both scripted fuzz gates, the Cargo-fuzz target
and seed, Gentle/profile-analyzer, and every named gate or regeneration script.
Tracked-source search finds retired names only in their durable documentation.

## Identity and accounting

The width source, allocation source, pure-memo source, and
`node-width-budgets.json` are byte-identical before and after rehoming. Layout
differs only in rustfmt import order. No fixture or benchmark baseline byte
changed.

Cumulative credited authored retirement is 1,980 lines:

- 46 duplicate parity reference-generation/run recipe lines;
- 1,446 one-time fixture-migration production and test lines; and
- 488 historical benchmark/prototype lines.

The relocated 284-line reference implementation, 786-line transaction owner,
155 ordinary-publication test lines, and 583 benchmark/baseline lines are moves
and receive no deletion credit. Generated root and standalone lockfile churn is
also excluded. Raw child implementation `--numstat` is +4,025/-3,132, but that
mixed tracked count is not authored retirement accounting. The closeout's
exhaustive-match and focused coverage repair is likewise not retirement credit.

## Fresh verification

The audit found one post-child integration defect: `state_budgets` did not
handle `ControlSequenceKind::Internal`. It now uses canonical tag `2`, matching
production state hashing, and a separate internal-control-sequence projection
workload executes that branch without changing the retained ordinary workload.
Every standalone benchmark package then compiled uncapped with six Cargo jobs.
Under 512 MiB, all command allocation rows, source-descriptor rows, shipout rows,
pure-memo rows, every layout row, the ordinary and internal control-sequence
projection rows, accepted-edit scaling, the assertion-bearing provenance row,
and the snapshot enforcement gate passed. The retained
dependency and format-cache diagnostics passed under the 1 GiB tooling cap.

Refexec passed 3 tests, parity's `reference-tools` resolution passed 21,
fixturegen passed 4 library plus 21 program tests, the selected `check-tools`
steps and provisioning self-test passed, and the live TFtoPL cross-check passed.
The 1,000-edit incremental fuzz gate and all 19 effectful rollback/commit tests
at 10,000 proptest cases passed under 512 MiB. `cargo-fuzz` is not installed on
this host, so its retained target/seed were audited but no fuzz execution is
claimed.

The unchanged width gate reproduced `umber2-9508`: 1,636.783 ns,
100,107.397 ns, and 108,590.493 ns exceed the foreign Apple/aarch64 limits.
The unchanged allocation gate reproduced `umber2-dtis`: line breaking uses 13
allocations, deep sublists use 340,041 allocations and 50,653,668 bytes, and
flat math uses 606,616 bytes; alignment and deep-choice remain within their
ceilings. No baseline was written. The complete native suite was compiled
uncapped and passed under 1 GiB; uncapped `scripts/check.sh` passed all four
gates.
