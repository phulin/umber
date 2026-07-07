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
UPDATE_FIXTURES=1 cargo test -p test-support --test hello
```

The update run rewrites missing or mismatched expected files, then panics.
Rerun without `UPDATE_FIXTURES` before committing.

## Cargo Test Scope

Keep `cargo test --workspace --tests` fast. Long or full-corpus parity runs
must stay out of cargo tests; the Conformance epic will own those runs in
`scripts/parity.sh`.
