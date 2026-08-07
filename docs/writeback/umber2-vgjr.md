# Architecture simplification portfolio closeout

Issue: `umber2-vgjr`

Integrated tree: `bd50a474138ec0f13f3c76caf6453113872fefd0`

## Closure audit

All 18 selected program epics, all 29 direct children, and all 99 portfolio
descendants are closed. Every program receipt names its surviving authority,
retired predecessor or explicit compatibility disposition, exact tested tree,
proof gates, and category-separated accounting. The final audit found no
stale open or in-progress child and no permanent migration-only dual
authority.

The authority results are:

| Program | Surviving authority                                                             | Retired predecessor or owner disposition                                                              |                                                                        Final measured result |
| ------: | ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------: |
|       1 | `execute_operation`/`apply_operation`, `AssignmentCommitter`                    | Four entry shapes, shadow harness, `CommandRuntime`, and pre-commit classifier retired                |                                                    Authored Rust `+3,819/-4,617`, net `-798` |
|       2 | Oracle views, `finalize_once`, comparison policy                                | Duplicate traversal, finalization, projection, parse, and accounting walks retired                    |                                                    Authored Rust `+2,002/-1,636`, net `+366` |
|       3 | `ResourceLifecycle`, `VerifiedDownloader`, `BlobStore::resolve_entry`           | Duplicate acquisition/admission paths and non-driving Rust planes retired                             |                                                    Authored Rust `+1,079/-1,747`, net `-668` |
|       4 | `EffectJournal`, `RevisionOutputPatch`, `RevisionTransaction`/`RevisionPayload` | Paragraph replay and duplicate revision/effect/artifact owners retired                                |                                                      Authored Rust `+818/-1,032`, net `-214` |
|       5 | Iterative artifact codec and explicit-frame geometry walkers                    | Recursive codec/geometry paths, streamed fresh-DVI path, and executor materializer retired            |        Production Rust `+1,505/-2,082`, net `-577`; tracked total `+2,225/-2,182`, net `+43` |
|       6 | `tex_out::pdf::finalize_pdf`                                                    | Umber finalizer and differential self-oracle retired                                                  | Production Rust `+6,412/-10,091`, net `-3,679`; tracked total `+6,723/-10,227`, net `-3,504` |
|       7 | `RenderDocument` and JavaScript `HtmlPatchMount`                                | Duplicate standalone producer and unused Rust receiver retired                                        |                          Production `+908/-1,607`, net `-699`; proof `+531/-210`, net `+321` |
|       8 | Node schema and sole production frozen decode                                   | Alternate logical schema, test restore, and public expansion façade retired                           |                                                  Authored Rust `+1,412/-3,248`, net `-1,836` |
|       9 | `tex-command::primitives`                                                       | State, executor, and Umber spelling/default inventories retired                                       |                                Authored Rust `+2,475/-1,272`, net `+1,203`; forecast retired |
|      10 | Math transaction, `ParagraphTape`, neutral metrics                              | Shadow math arena, repeated topology, and local metric accumulators retired                           |                           Production `+1,099/-494`, net `+605`; proof `+283/-184`, net `+99` |
|      11 | `FontMetrics`, borrowed MATH table, realized identity                           | Raw TFM/MATH graphs, repeated decode, and duplicate projections retired                               |                                                      Production Rust `+714/-855`, net `-141` |
|      12 | `FixtureCase`, schema V2, TeX82 default, fixture transaction                    | Repeated catalogues, validators, serializers, and mutation paths retired                              |     Authored/config `+1,676/-1,173`, net `+503`; declarative `+6,834/-21,244`, net `-14,410` |
|      13 | Wire DTOs, `SessionDriver`, `WorkerRpcClient`, catalogue/publisher authorities  | Manual wire mirrors, duplicate drivers, JavaScript catalogue semantics, and publisher shadows retired |                                            Authored source/proof `+3,880/-3,633`, net `+247` |
|      14 | Biber draft/freeze, classic VM, XML projection, output router                   | Intermediate mutable, lexer, READ, cache, diagnostic, XML, and output models retired                  |                  Production `+1,334/-1,743`, net `-409`; tests `+3,017/-7,429`, net `-4,412` |
|      15 | Active case owners and executable source audit                                  | Dormant islands and proven duplicate scaffolding retired; unique cases retained                       |                                                      Authored Rust `+879/-1,042`, net `-163` |
|      16 | `PdfQuery`, `normalize_structure`, `ValidPdfFixture`                            | `PdfProbe` graph and ordinary handwritten valid writer retired; raw adversarial writer retained       |                                  Authored Rust `+1,751/-1,448`, net `+303`; forecast retired |
|      17 | `GeneratedTransaction` and three shape-safe maps                                | Multistage build plan and generic public layer model retired                                          |                      Rust `+706/-1,578`, net `-872`; tracked total `+885/-1,689`, net `-804` |
|      18 | Fixturegen reference/publication kernel and classified retained tools           | Only owner-approved migrations and historical surfaces retired                                        |                     1,980 authored lines credited; 1,808 moved lines and lock churn excluded |

The program sections and their linked issue writebacks contain the finer
production, proof, documentation, declarative/generated, lockfile,
compatibility, and binary categories. The portfolio owner accepts every
measured result above and retires every remaining planning-range variance. No
future or historical deletion is credited, and no shortfall is carried.

## Integration-interval reconciliation

The rename-aware diff from plan commit `0bf7219ea` to the integrated tree
changes 828 paths by 57,017 additions and 86,766 deletions:

| Category                                         | Additions | Deletions |     Net |
| ------------------------------------------------ | --------: | --------: | ------: |
| Authored Rust/JavaScript/TypeScript/Python/shell |    40,007 |    63,660 | -23,653 |
| Documentation and guidance                       |     6,597 |     1,247 |  +5,350 |
| Declarative fixtures and configuration           |     7,569 |    21,607 | -14,038 |
| Generated lockfiles                              |     2,844 |       252 |  +2,592 |

Two binary paths are excluded from line totals. This 215-commit interval also
contains independently tracked repairs and non-portfolio work, so it is an
exact repository reconciliation rather than program deletion credit.

## Final acceptance

The superseding safety policy was used throughout this final run:
`CARGO_BUILD_JOBS=6`, uncapped Cargo builds, finite cgroup-capped runtime,
normal test concurrency, no `prlimit`, and no overlapping Cargo process.

- `cargo test -q --tests --no-run` passed uncapped with six build jobs.
- `cargo test -q --tests` passed under `MemoryMax=1G`, zero swap, and an
  1,800-second timeout in 26.752 seconds. This routine suite includes the
  active Story exact-DVI owner.
- `e2e_conformance_story_canonical` was also compiled uncapped and passed
  independently under `MemoryMax=512M`, zero swap, and a 900-second timeout.
- The standalone `benchmarks/tex-typeset` `layout_allocations` binary built
  uncapped in locked release mode with six jobs. Under `MemoryMax=512M`, zero
  swap, and a 600-second timeout it passed in 23.294 seconds with unchanged
  rows: alignment `1,023/191,840`, line breaking `12/384,619,648`, deep choice
  `9/9,758,056`, deep sublist `160,044/23,855,088`, and flat math
  `10/508,360` allocations/bytes.
- `scripts/check.sh` passed uncapped with six build jobs: dprint, Biome,
  rustfmt, and both clippy resolutions.

An initial full-suite service launch resolved the wrong Cargo environment and
exited before tests at 612 KiB. It is excluded from acceptance; the corrected
explicit-toolchain cgroup launch produced the passing result above.
