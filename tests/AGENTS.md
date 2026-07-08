# Tests Guidance

`tests/corpus` holds committed inputs and expected reference outputs for
fast differential tests.

## Corpus Layout

Each case input lives at:

```text
tests/corpus/<area>/<case>.tex
```

Place any small area-local support files beside the case input unless a
future test explicitly needs a subdirectory. Expected files use:

`tests/corpus/exec` contains fast execution-core parity cases. These compare
`umber run` output with normalized `pdftex` logs through the shared
`test_support::normalize::exec_log` helper.

`tests/corpus/typeset` contains fast box/list dump parity cases for the
typesetting layer. These compare `umber run --show-fixtures` output with
normalized `pdftex` logs through the shared
`test_support::normalize::box_dump` helper; that helper uses the same
diagnostic-log normalizer as `exec_log`.

```text
<case>.expected.<kind>
```

For example, `tests/corpus/hello/hello.expected.log` is the normalized
reference log for `tests/corpus/hello/hello.tex`.

## Fixture Updates

Use `test_support::assert_matches_fixture(area, case, kind, actual)` for
fixture assertions. When output changes intentionally, regenerate the
fixture by running the focused test with:

```bash
UPDATE_FIXTURES=1 cargo test -p test-support hello_reference_log_matches_fixture
```

The update run rewrites missing or mismatched expected files, then panics.
Rerun without `UPDATE_FIXTURES` before committing.

For typeset box-dump fixtures, run:

```bash
UPDATE_FIXTURES=1 cargo test -p umber --test it run_typeset_corpus_matches_pdftex_box_dumps
cargo test -p umber --test it run_typeset_corpus_matches_pdftex_box_dumps
```

## Cargo Test Scope

Keep `cargo test --workspace --tests` fast. Long or full-corpus parity runs
must stay out of cargo tests; the Conformance epic will own those runs in
`scripts/parity.sh`.

Font metric parity tests use an optional local TFM corpus under
`third_party/fonts/`, which is gitignored. Populate it from an ambient TeX
installation with:

```bash
scripts/fetch-font-corpus.sh
```

When those files are absent, the `tftopl` corpus cross-check prints a clear
skip message and returns success so the fast suite still runs on machines
without TeX fonts.

## Proptest Budgets

Replay-identity proptests use `PROPTEST_CASES` for their case budget. Leave
the default small enough for `cargo test --workspace --tests`; raise it for
local long runs, for example:

```bash
PROPTEST_CASES=1000 cargo test -p umber --test it replay_identity
cargo test -p umber --features shadow --test it replay_identity
```

Effectful rollback/commit fuzzing uses the same budget variable and is wired
through:

```bash
scripts/effectful-rollback-fuzz.sh
PROPTEST_CASES=1000 scripts/effectful-rollback-fuzz.sh
```

The script defaults to 10,000 generated cases and covers World effects,
pre-commit leak assertions, rollback state-hash identity, and committed-prefix
replay checks. Do not move that long run into default cargo tests.
