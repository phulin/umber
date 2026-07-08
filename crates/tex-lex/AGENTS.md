# tex-lex Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's eyes and mouth: line normalization, source frames, tokenization, token-list replay, and input-stack state.

## Crate Role

`tex-lex` turns physical input lines or frozen token lists into TeX tokens under the active catcode table. It owns `InputSource`, in-memory and world-backed source adapters, TeX line normalization, source-frame lexer state, token-list replay frames, conditional frames used while skipping input, and resumable summaries for input-stack state.

Use this crate for source-local mechanics: reading normalized logical lines, applying catcodes, delivering raw tokens, replaying token lists, and preserving enough input-stack state for snapshots/replay. Durable file identity and actual file reads belong to `World`; expansion semantics belong to `tex-expand`.

## Boundaries

- Do not open files directly; accept an `InputSource` or content supplied by `World`.
- Do not evaluate meanings or expand macros here; token semantics begin in `tex-expand` and `tex-exec`.
- Do not mutate raw substores or reach around the aggregate state APIs for catcodes or input summaries.
- Keep lexer state resumable when adding fields to source, token-list, or condition frames.

## File Map

- `AGENTS.md`: crate-local guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: crate manifest for the `tex-lex` library and its `tex-state` dependency.
- `src/lib.rs`: input sources, line normalization, source-frame lexer state, tokenization, input stack, token-list replay, condition frames, and resumable summaries.
- `src/tests.rs`: crate-internal coverage for line handling, TeX tokenization rules, input-stack replay, condition frames, summaries, and lexer edge cases.
- `src/tests/input_lines.rs`: focused tests that memory and world-backed file inputs share TeX line normalization behavior.

## Validation

Run `cargo test --tests -p tex-lex` for lexer/input-stack changes. If summaries or replay behavior change, also run replay-oriented tests in `umber` and state snapshot tests that depend on input resumability.
