# umber2-vgjr.9.3 — primitive catalogue consumer migration

Authority: [`tex-command`'s integrated primitive catalogue](../../crates/tex-command/src/primitives), completed by implementation commits `9ee00dd5a` and `664d1a8b9`.

All primitive consumers now derive their static facts from the catalogue.
`tex-state` keeps typed parameter cells but no spelling tables; restoration
traces resolve their names from the installed primitive registry.
`tex-command` installs enum commands, parameter cells, page and internal
quantities, `nullfont`, and frozen `endwrite` from profile views in the original
INITEX order and reconstructs the same registry without shadowing loaded-format
meanings. `tex-exec` retains handwritten behavioral dispatch and only thin
installation wrappers plus catalogue-backed tracing-name lookup. Umber's
pdfTeX layer delegates installation and restoration, derives the exact sorted
158-name set, and applies typed parameter defaults from the catalogue.

The retired predecessors are `tex-state`'s integer, dimension, glue, and token
parameter name tables; `tex-exec`'s TeX82/e-TeX parameter, page, special,
and tracing-name tables; Umber's pdfTeX 158-name, parameter meaning/default,
dimension/token, and internal-quantity tables; its pristine-registry rebuild;
and migration-only predecessor parity loops. The exact source-checklist test
now compares the catalogue projection directly. Allocation-sensitive
`nullfont` and `endwrite` construction remains a store-local installation seam,
not a second inventory. Execution bodies remain handwritten.

Focused capped evidence covers the integrated exceptional catalogue and exact
158 names, all parameter view/default/alias cases, primitive registry and
format startup, fresh INITEX category state, loaded frozen meanings, and all 52
observation identity tests. The uncapped full workspace `--no-run` build and
the full routine suite under `MemoryMax=1G` passed. The routine suite includes
689 `tex-command` tests and 469 Umber library tests (440 passed, 29 ignored),
with all workspace binaries green. `scripts/check.sh` passed all four gates
under the 1 GiB cgroup.

Implementation commits `9ee00dd5a` and `664d1a8b9` have 636 additions and
1,238 deletions in authored Rust: 602 lines of measured net deletion. Documentation and guidance
are recorded separately; no declarative/generated or binary-fixture deletion
is credited. No linked discovery was required.
