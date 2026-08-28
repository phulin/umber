# tex-typeset Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns pure TeX typesetting kernels.

## Crate Role

`tex-typeset` contains list-in/list-out algorithms such as badness calculation, hpack/vpack/vtop packing, line-breaking support, and post-processing over node lists. Its public entry points read immutable state through narrow traits, copy required parameters into plain values, perform TeX arithmetic, and return packed boxes, diagnostics, or transformed lists. The crate does not implement those traits for `Universe`; an execution-layer adapter must hold the admitted state/page borrows and resolve typed coordinates for the duration of one call.

Use this crate for layout algorithms whose correctness can be tested as pure functions over node/font/glue inputs. Stomach code in `tex-exec` should prepare lists and apply side effects before or after calling into these kernels.

## File Map

- `AGENTS.md`: local guidance for future agents working in this crate.
- `Cargo.toml`: crate manifest, local dependencies, and workspace lint configuration.
- `src/lib.rs`: public crate surface, the execution-adapter-facing `TypesetState` trait, `badness`, and packing exports.
- `src/alignment.rs`: pure detached alignment column/span width planning.
- `src/alignment/tests.rs`: unit tests for independent alignment width planning.
- `src/expansion.rs`: pure pdfTeX font-expansion validation, capacity, final-ratio, and discrete-step arithmetic.
- `src/math/`: pure Appendix G math-list conversion helpers, including the iterative borrowed-coordinate choice/view pass driver, style transitions, math parameter snapshots, compound fractions/operators/radicals/delimiters/accents, script placement, a detached postorder native-node transaction, shared replay recipes for nested-list observations, and inter-noad spacing. Canonical source leaves remain `(PageListId, index)` coordinates; only genuinely rewritten noads and generated layout nodes are owned drafts. Nested-list preparation retains direct evidence plus child recipe references; only root demand flattens the transitive event stream.
- `src/math/model/tests.rs`: direct TeX82 math-field, noad-kind, fraction, choice-arm, and style-model tests.
- `src/math/convert/tests.rs`: direct TeX82 first/second mlist-pass, choice, bin, nonscript, spacing, delimiter, and penalty tests.
- `src/math/operators/tests.rs`: direct TeX82 noad-construction, limits, italic-correction, script, fraction, radical, accent, and rule tests.
- `src/math/variants.rs`: deterministic OpenType MATH size-variant selection and horizontal/vertical glyph-assembly planning.
- `src/math/variants/tests.rs`: connector, overlap, extender-repetition, and malformed-construction tests.
- `src/metrics.rs`: neutral metric-event IR and shared horizontal, vertical, and wide accumulators; domain modules retain glue, breakpoint, font-expansion, and observation policy.
- `src/test_state.rs`: test-only value projection over a typed page arena, copied font/parameter values, and narrow typesetting traits; it is not a runtime or `Universe` facade.
- `src/math/arithmetic.rs`: shared checked scaled-arithmetic guards for Appendix G.
- `src/math/rebox.rs`: shared TeX82 §715 exact-width math-box centering and vertical-source packaging.
- `src/packing.rs`: hpack/vpack/vtop kernels over page-arena list coordinates,
  pack parameters, measurements, glue setting, and diagnostics.
- `src/packing/tests.rs`: unit tests for badness, packing dimensions, glue settings, diagnostics, and vtop behavior.
- `src/vertical_break.rs`: pure TeX.web `vert_break` page/split breakpoint costing over immutable vertical lists.
- `src/vertical_break/tests/planned.rs`: direct TeX82 vertical-break depth, cost, forced-end, and tie-policy tests.
- `src/linebreak/mod.rs`: line-breaking API, line-shape types, pass orchestration,
  breakpoint search, and demerit scoring. Active routes keep a compact stable
  break-site index and reuse the immutable tape's successor position and width
  metrics; production hands its arena coordinate, scalar materialization
  actions, and optional diagnostic projection to the executor's retained-range
  sink rather than materializing nodes.
- `src/linebreak/post.rs`: pure slice/owned post-line-break adapters for broken
  lines, skips, migrated discretionary material, and penalties. Production
  arena tapes must not use their owned-node channel.
- `src/linebreak/tests.rs`: unit tests for line dimensions, break selection, hyphenation hooks, penalties, and post-break output.
- `src/linebreak/widths.rs`: line width accumulation, prefix width tables, glue stretch/shrink accounting, and line badness.
- `tests/production_traits.rs`: public-boundary smoke tests proving packing,
  line-breaking, and math conversion through a typed page-arena adapter with
  copied parameter values and no runtime owner facade.

## Boundaries

- Do not access or mutate `Universe` from this crate. Runtime consumers provide
  a narrow adapter over an already admitted generation and the matching page
  arena.
- Do not handle primitive dispatch, grouping, mode transitions, file effects, or artifact commits here.
- Keep font and durable parameter access through narrow immutable traits;
  page nodes carry glue values directly, and nested lists resolve only through
  the borrowed page arena.
- Copy glue parameters into `GlueSpec` while taking a math-parameter snapshot;
  generation-branded `GlueId<G>` values do not cross the pure-kernel trait.
- Treat `PageListId` as a coordinate only. Resolve it through
  `TypesetState::page_nodes` before reading length or contents.
- Preserve TeX.web arithmetic and badness rules exactly; route shared fixed-point operations through `tex-arith`/`tex-state::scaled` as appropriate.

## Validation

Run `cargo test --tests -p tex-typeset` for local algorithm changes. If a packing or line-breaking change is reached through execution primitives, also run the relevant `tex-exec` parity tests.
