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
`umber run` output with committed normalized `pdftex` log fixtures through the
shared `test_support::normalize::exec_log` helper. Normal test runs read the
committed `<case>.expected.log` files; `UPDATE_FIXTURES=1` regenerates those
fixtures from a live reference engine.

`tests/corpus/typeset` contains fast box/list dump parity cases for the
typesetting layer. These compare `umber run --show-fixtures` output with
committed normalized `pdftex` log fixtures through the shared
`test_support::normalize::box_dump` helper; that helper uses the same
diagnostic-log normalizer as `exec_log`. In this mode, `umber` writes only the
collected terminal/log diagnostic text to stdout and skips the CLI's extra
final `World` effect commit. TeX shipouts still commit their own effect prefix,
so stream whatsits shipped by `\shipout` or final cleanup can materialize
output files even under `--show-fixtures`; pending immediate stream effects do
not materialize because their final commit is skipped.

`tests/corpus/dvi` contains committed TeX source fixtures for full-pipeline
DVI parity plus committed `<case>.expected.dvi` reference fixtures. The
`umber` cargo DVI corpus test runs every `.tex` case in this area against its
committed expected DVI so default tests remain hermetic. `scripts/parity.sh`
still copies every source plus pinned CM TFMs into a temporary run directory,
runs `umber run <case>.tex --dvi actual.dvi`, then asks `tools/refexec` to run
the live reference engine and byte-compare DVI output with only preamble
comment payload normalization.

`tests/corpus/page` contains page-builder-focused DVI parity fixtures. It is
run by the same `scripts/parity.sh` DVI comparison loop. The `umber` cargo
page-corpus test also runs every `.tex` case in this area against committed
`<case>.expected.dvi` reference fixtures. Page cases should use small
primitive-only preambles that pin plain-format defaults such as `\output`,
`\maxdepth`, and interline glue whenever pdfTeX plain defaults would
otherwise leak into byte output.

`tests/corpus/tex_exec` contains small normalized pdfTeX reference observations
used by `tex-exec` crate-internal tests for grouping, after-token ordering,
magnification diagnostics, and box-register behavior.

`tests/corpus/tex_exec_io` contains small pdfTeX-derived file-effect and DVI
special-payload observations used by `tex-exec` I/O and shipout tests.

`tests/corpus/math` contains primitive-only math DVI parity fixtures plus
committed `<case>.expected.dvi` reference fixtures. Cases share
`math_preamble.inc`; keep that include free of `plain.tex` dependencies and
keep individual `.tex` cases small. The cargo test runs each case against its
committed DVI fixture; the live parity harness runs the reference engine in
INITEX mode for this area, copies the shared include beside each case, and
pins `cmr10`, `cmmi10`, `cmsy10`, and `cmex10` TFMs so text/script/
scriptscript family selection observes the same metrics as Umber.

`tests/corpus/align` contains alignment-focused DVI parity fixtures for
`\halign`, `\valign`, spans, omission, `\noalign`, nested alignment, and
display alignment, with committed `<case>.expected.dvi` reference fixtures.
The cargo test runs each case against its committed DVI fixture, and
`scripts/parity.sh` runs the same area with the same pinned CM TFMs as the
other DVI corpora; keep cases primitive-only.

`tests/corpus/leaders` contains leader-focused DVI byte-parity fixtures for
`\leaders`, `\cleaders`, and `\xleaders`, with committed
`<case>.expected.dvi` reference fixtures. The cargo test runs each case
against its committed DVI fixture, and `scripts/parity.sh` keeps the explicit
live-reference parity tier.

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

For `umber` execution and typeset diagnostic fixtures, run:

```bash
UPDATE_FIXTURES=1 cargo test -p umber --test it run_exec_corpus_matches_pdftex_diagnostics
UPDATE_FIXTURES=1 cargo test -p umber --test it run_typeset_corpus_matches_pdftex_box_dumps
```

The `umber` DVI/page/math/align/leaders cargo tests are committed-fixture
checks only and do not regenerate DVI fixtures. Until the unified regeneration
script lands, regenerate DVI fixtures with an explicit live reference workflow
outside default cargo tests, then rerun the relevant cargo test:

```bash
cargo test -p umber --test it run_dvi_corpus_matches_committed_dvi
cargo test -p umber --test it run_page_corpus_matches_committed_dvi
cargo test -p umber --test it run_math_corpus_matches_committed_dvi
cargo test -p umber --test it run_align_corpus_matches_committed_dvi
cargo test -p umber --test it run_leaders_corpus_matches_committed_dvi
```

For `tex-exec` pdfTeX-derived micro fixtures, run:

```bash
UPDATE_FIXTURES=1 cargo test -p tex-exec --lib grouping_parity
UPDATE_FIXTURES=1 cargo test -p tex-exec --lib io::
```

Fixture update runs require a live reference TeX (`pdftex` or `UMBER_REF_TEX`)
and may stop after rewriting the first changed fixture. Repeat the focused
command until it passes, then rerun without `UPDATE_FIXTURES` before
committing:

```bash
cargo test -p umber --test it run_typeset_corpus_matches_pdftex_box_dumps
```

For live reference checks without rewriting fixtures, run `scripts/parity.sh`
or set `UMBER_LIVE_REF=1` on the focused cargo test. Ordinary `cargo test
--tests` must not set `UMBER_LIVE_REF` and must not require TeX tools on
`PATH`.

## Cargo Test Scope

Keep `cargo test --workspace --tests` fast. Long or full-corpus parity runs
must stay out of cargo tests; the Conformance epic will own those runs in
`scripts/parity.sh`.

The fast/default tier is `cargo test --tests`, `cargo test --workspace --tests`,
and `scripts/check.sh`. It reads committed fixtures and must run without TeX
tools on `PATH`; keep warmed `cargo test --tests` under the documented
10-second target in `docs/testing_policy.md`. The slow parity tier is
`scripts/parity.sh`, which sets `UMBER_LIVE_REF=1` and runs live reference
diagnostic and byte-identical DVI corpus checks.

Font metric parity tests use an optional local TFM corpus under
`third_party/fonts/`, which is gitignored. Populate it from an ambient TeX
installation with:

```bash
scripts/fetch-font-corpus.sh
```

The live `tftopl` corpus cross-check runs only when `UMBER_LIVE_REF=1` is set.
When those files are absent in that explicit mode, it prints a clear skip
message and returns success so the fast suite still runs on machines without
TeX fonts.

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
