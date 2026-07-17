# bib-bst Guidance

Read the repository-level `AGENTS.md` and `docs/classic_bibtex_bst.md` at
commit `c676cfb0` before editing here. This crate owns only the pure,
host-neutral classic `.bst` lexer, parser, compiler, immutable programs, and
their bounded compilation cache. Do not add database preparation, VFS access,
output, or VM execution here; those are owned by the classic `bib-engine`
session and later BST VM issues.

## File Map

- `src/lib.rs`: public bounded compilation and cache boundary.
- `src/lexer.rs`: byte-aware tokens and bounded lexical recovery.
- `src/parser.rs`: all ten top-level commands and parser recovery.
- `src/program.rs`: immutable typed symbols, instructions, and compiled style.
- `src/compiler.rs`: phase validation, resolution, lowering, and limits.
- `src/cache.rs`: charged FIFO compilation cache with per-hit limit validation.
- `src/tests.rs`: crate-internal lexer/parser/compiler/cache tests.
