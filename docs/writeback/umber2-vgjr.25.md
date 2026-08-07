# umber2-vgjr.25 -- typesetting forecast reconciliation

## Owner decision

The portfolio owner accepts Program 10 at its measured 605 lines of net
production Rust growth and 99 lines of net proof/test Rust growth, for 704
lines of net authored Rust growth. The original 900--1,250 production and
700--950 conditional test reduction forecasts are retired. No unimplemented
deletion remains scheduled, and no moved, generated, declarative,
documentation, or total-line change is credited against either authored
category.

## Authority and caller audit

- `tex_typeset::math::MathLayout` is the sole detached postorder native-node
  transaction. The executor-private `commit_math_transaction` is its only
  publication seam. The retired reader, sink, builder, public sink API, and
  native-to-math-to-native source-leaf round trip remain absent.
- `tex_typeset::linebreak::ParagraphTape` is the sole analyzed paragraph
  authority. It owns the paired semantic and physical `NodeSequence`, legal
  break sites with wide prefix metrics, trace spans, and materialization
  actions. Production breaking, tracing, and executor materialization consume
  that tape; `LineBreakResult` has no parallel node, physical-node, or boundary
  fields.
- The retained `post_line_break` and `LineMaterializer::from_nodes` convenience
  path materializes caller-selected break decisions for pure focused tests. It
  performs no break search, prefix analysis, tracing, or executor projection,
  so it is not a competing paragraph authority. The owner explicitly preserves
  this compatibility and proof boundary.
- `MetricEvent`, `MetricsCursor`, `ListMetrics`, and `WideMetricTotals` are the
  neutral metric authority. Packing retains glue setting, perpendicular leader
  geometry, and diagnostics; line breaking retains font expansion, wide prefix
  subtraction, badness, and demerits; vertical breaking retains breakpoint,
  infinite-shrink, checked-overflow, and tie policy; Appendix G retains
  transaction topology and occurrence-ordered pack observations. These are
  explicit domain decisions, not duplicate generic accumulators.

The closed assertion ledger maps every assertion in the three removed
aggregate cases to an active case-level owner and retains the unique
artificial-end vertical tie case. The later typesetting/browser compaction
ledger independently found no further assertion-complete repetition. Active
owners retain 20,000-depth math and discretionary bounds, 100,000-node linear
tape storage, paired semantic/physical diagnostics, source-box geometry,
occurrence ordering, missing-family recovery, wide-prefix and vertical
overflow, exact bytes, and packing/performance differentials. Deleting or
merging them would weaken a distinct contract rather than remove duplication.

## Exact accounting

Independent classification of implementation commits `c0b05de33`,
`082687614`, `cb2f9fb60`, `12e8be260`, `c93415e2b`, `5cf76deb8`, and
`56995b38f` reproduces the closeout categories.

| Category         | Additions | Deletions |  Net |
| ---------------- | --------: | --------: | ---: |
| Production Rust  |     1,099 |       494 | +605 |
| Proof/test Rust  |       283 |       184 |  +99 |
| Authored Rust    |     1,382 |       678 | +704 |
| Documentation    |       158 |         7 | +151 |
| Declarative maps |         7 |         3 |   +4 |
| Complete program |     1,547 |       688 | +859 |

Summed per-commit numstat includes 22 added and 22 deleted lines of neutral
intermediate churn that cancel in the program endpoint. Rust numstat also
contains ten added and ten deleted moved lines; excluding that move reproduces
the authored gross counts without changing the +704 net result. No moved line,
generated source, fixture, binary, documentation, declarative map, or gross
predecessor deletion is converted into production or proof/test credit.

## Verification and resolved performance dependency

On the documentation-only reconciliation tree based on `8048c0453`, the
complete native suite and standalone typesetting performance scopes compiled
uncapped with `CARGO_BUILD_JOBS=6`. All 172 `tex-typeset` tests passed with six
test threads under `MemoryMax=512M`, covering the named deep math,
discretionary, tape, semantic/physical, geometry, observation, overflow, and
differential owners. The complete `cargo test -q --tests` tier passed with six
test threads under `MemoryMax=1G`; its active Story gate consumed the local
reference oracle and passed byte-exact DVI comparison. Uncapped
`scripts/check.sh` passed all four gates. Every runtime had a finite timeout,
and no command used `prlimit` or serialized the test harness.

The initial unchanged deterministic `layout_allocations` performance gate
identified a real blocker: `linebreak_long_paragraph` used 13 allocations
against a ceiling of 12, while `math_deep_submlist_stack` used 340,041
allocations and 50,653,668 bytes against ceilings of 180,000 and 42,000,000.
The owner declined to weaken those budgets or absorb a nontrivial repair into
this accounting audit. P1 bug `umber2-vgjr.27` therefore owned the repair.

That dependency is resolved at exact tree
`27cb5476c4edcbb0ad0411afc9f1b5c14ab33aa0`. The unchanged 512 MiB allocation
gate now passes every row: long paragraph at 12 allocations and 384,619,648
bytes; deep sublist at 160,044 and 23,855,088; alignment at 1,023 and 191,840;
deep choice at nine and 9,758,056; and flat math at ten and 508,360. The repair
keeps breakpoint traces inside the owning `BreakSite`, reuses
transaction-private Appendix G planning and conversion buffers, and uses the
existing empty `Replay` representation. It adds no second paragraph, math,
metric, observation, or publication authority, and changes neither benchmark
ceilings nor Program 10 accounting.

Fresh closeout verification compiled the complete native scope uncapped with
`CARGO_BUILD_JOBS=6` and `--no-run`, then compiled the release allocation gate
uncapped. The prebuilt allocation binary and all 172 `tex-typeset` tests passed
under `MemoryMax=512M`, `MemorySwapMax=0`, and finite 1,800-second timeouts.
The complete native suite passed with six test threads under `MemoryMax=1G`,
`MemorySwapMax=0`, and the same timeout, including the active Story exact-DVI
comparison. Final `scripts/check.sh` passed all four gates uncapped. The owner
decision, sole authorities, explicit domain policies, assertion ledger, exact
accounting, compatibility proof, and unchanged allocation budgets are all
satisfied; no shortfall remains to carry forward.
