# tex-expand Guidance

Read the repository-level `AGENTS.md` before editing here.

## Crate Role

`tex-expand` is a retired compatibility crate. Its compiled facade may forward
canonical primitive-installation entry points from `tex-command`, but it must
not implement command behavior or depend on `tex-lex`. The workspace member
remains until `umber2-johp.15` removes the crate identity.

Retired Umber expansion behavior and tests are not correctness oracles. New
command behavior and source-backed tests belong in `tex-command`.

## File Map

- `Cargo.toml`: the dependency-light retired facade manifest.
- `src/lib.rs`: canonical `tex-command` installation forwards only.
- `AGENTS.md`: this ownership contract.
