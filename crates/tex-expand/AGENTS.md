# tex-expand Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns TeX's gullet: expansion, expandable primitives, macro argument scanning, conditionals, and reusable value scanners.

## Crate Role

`tex-expand` implements the `get_x_token`-style expansion loop over `tex-lex::InputStack`. It reads meanings through `tex-state::Universe`, expands expandable primitives and macros, manages conditional skipping, replays frozen token lists through the lexer stack, and provides shared scanners for integers, dimensions, glue, token lists, and expansion-derived textual values.

Use this crate for behavior that is defined before stomach execution sees an unexpandable token. Expansion should be explicit about hooks for file input, job names, and other driver-owned facts; the crate should not open files or perform host effects itself.

## Boundaries

- Do not mutate state except through the aggregate state capabilities exposed by `Universe`.
- Do not implement unexpandable primitive side effects here; those belong in `tex-exec`.
- Do not bypass `tex-lex` for input-stack or token-list replay behavior.
- Keep scanner arithmetic exact and shared with `tex-arith`/`tex-state::scaled` where appropriate.

## File Map

- `AGENTS.md`: crate-local guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: crate manifest, workspace lint inheritance, and tex-expand dependencies.
- `src/args.rs`: macro-call parameter matching and argument token-list freezing.
- `src/args_tests.rs`: unit coverage for macro argument scanning behavior.
- `src/conditionals.rs`: conditional stack transitions and skipped-branch scanning for `\if...`, `\else`, `\or`, and `\fi`.
- `src/dispatch.rs`: expandable token dispatch, hook-aware primitive handling, and expansion result routing.
- `src/lib.rs`: public crate API, core expansion loop types, hooks, errors, and primitive installation.
- `src/primitives.rs`: implementations for expandable primitive helpers such as `\expandafter`, `\csname`, and `\input` name scanning.
- `src/scan.rs`: reusable macro definition token scanning for `\def`/`\edef`-style callers.
- `src/scan/tests.rs`: unit tests for macro definition token scanning.
- `src/scan_dimen.rs`: expanded TeX dimension scanning, unit conversion, diagnostics, and internal-dimension reads.
- `src/scan_dimen/tests.rs`: unit tests for dimension scanning edge cases.
- `src/scan_glue.rs`: expanded glue and muglue scanning, including stretch/shrink component parsing.
- `src/scan_glue/tests.rs`: unit tests for glue and muglue scanner behavior.
- `src/scan_helpers.rs`: shared expanded-token helpers for spaces, keywords, register indexes, signs, and filler.
- `src/scan_int.rs`: expanded TeX integer scanning, numeric syntax, diagnostics, and internal-integer reads.
- `src/scan_int/tests.rs`: unit tests for integer scanner behavior.
- `src/tests.rs`: crate-level tests for expansion dispatch and public expansion behavior.
- `src/values.rs`: rendering and expansion of value-producing primitives such as `\the`, `\meaning`, and token text.
- `tests/capability_boundaries.rs`: trybuild integration test enforcing scanner helper capability boundaries.
- `tests/ui/scanner_helper_input_open_forbidden.rs`: compile-fail fixture proving scanner helpers cannot require input-opening capabilities.
- `tests/ui/scanner_helper_input_open_forbidden.stderr`: expected compiler diagnostics for the scanner helper compile-fail fixture.

## Validation

Run `cargo test --tests -p tex-expand` for expansion and scanner changes. For scanners used by assignments, also run focused `tex-exec` tests because execution code depends on their exact edge-case behavior.
