# tex-typeset Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns pure TeX typesetting kernels.

## Crate Role

`tex-typeset` contains list-in/list-out algorithms such as badness calculation, hpack/vpack/vtop packing, line-breaking support, and post-processing over node lists. Its public entry points read immutable state through narrow traits, copy required parameters into plain values, perform TeX arithmetic, and return packed boxes, diagnostics, or transformed lists without mutating `Universe`.

Use this crate for layout algorithms whose correctness can be tested as pure functions over node/font/glue inputs. Stomach code in `tex-exec` should prepare lists and apply side effects before or after calling into these kernels.

## File Map

- `AGENTS.md`: local guidance for future agents working in this crate.
- `Cargo.toml`: crate manifest, local dependencies, and workspace lint configuration.
- `src/lib.rs`: public crate surface, `TypesetState`, `badness`, and packing exports.
- `src/math/`: pure Appendix G math-list conversion helpers, including style transitions, math parameter snapshots, compound fractions/operators/radicals/delimiters/accents, script placement, and inter-noad spacing over owned hlist output.
- `src/packing.rs`: hpack/vpack/vtop kernels, pack parameters, measurements, glue setting, and diagnostics.
- `src/packing/tests.rs`: unit tests for badness, packing dimensions, glue settings, diagnostics, and vtop behavior.
- `src/vertical_break.rs`: pure TeX.web `vert_break` page/split breakpoint costing over immutable vertical lists.
- `src/linebreak/mod.rs`: line-breaking API, line-shape types, pass orchestration, breakpoint search, and demerit scoring.
- `src/linebreak/post.rs`: post-line-break list surgery for broken lines, skips, migrated disc material, and penalties.
- `src/linebreak/tests.rs`: unit tests for line dimensions, break selection, hyphenation hooks, penalties, and post-break output.
- `src/linebreak/widths.rs`: line width accumulation, prefix width tables, glue stretch/shrink accounting, and line badness.

## Boundaries

- Do not mutate `Universe` from this crate.
- Do not handle primitive dispatch, grouping, mode transitions, file effects, or artifact commits here.
- Keep font and glue access through narrow immutable traits so algorithms remain easy to test and reuse.
- Preserve TeX.web arithmetic and badness rules exactly; route shared fixed-point operations through `tex-arith`/`tex-state::scaled` as appropriate.

## Validation

Run `cargo test --tests -p tex-typeset` for local algorithm changes. If a packing or line-breaking change is reached through execution primitives, also run the relevant `tex-exec` parity tests.
