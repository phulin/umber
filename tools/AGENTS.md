# Tools Guidance

`tools/refexec` is an opt-in host-side regeneration utility: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default the tool locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point fixture regeneration at a different reference binary, such as a specific TeX Live installation. Exact DVI normalization/comparison is owned by `test-support`; `refexec` re-exports and uses that shared contract for its CLI comparison paths.

`tools/fixturegen` is the script-owned fixture regeneration tool used by `scripts/regen-fixtures.sh` for text/native fixtures, pinned pdfTeX/Poppler PDF parity fixtures, and the explicit live font check. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/fixturegen/Cargo.toml`. It may invoke `refexec`, `umber`, `pdftex`, `pdftoppm`, and `tftopl`, but cargo tests must not build or run it.

Its `--classic-bibtex-differential` mode is called only by the `bibtex` branch
of `scripts/regen-fixtures.sh`. It generates a fixed, bounded seed corpus of
legal `.bst` programs, stages each case without host lookup, and compares
reference/Umber status, BBL, and BLG bytes. Failures are preserved under
`target/bst-differential/failures/` with their exact seed and inputs.

`refexec` also wraps `tftopl` for the font metric check owned by `tools/fixturegen`. When running that tier, it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point regeneration at a specific TeX installation.

`tools/corpus-sync` is the external document acquisition tool used by `scripts/setup-conformance-tests.sh`. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/corpus-sync/Cargo.toml`. It reads the line-oriented `tests/corpus-manifest.txt`, fetches exact support inputs and runnable documents into gitignored `third_party/corpus/`, verifies SHA-256, and treats cached hash matches as a no-op. Once setup is complete, conformance tests consume only local inputs and require no network access. Do not normalize line endings or commit fetched corpus files; licensing determinations live in the manifest notes.

`tools/texlive-wasm-publish` is a standalone release tool for browser TeX Live assets. It verifies every configured TEXMF root against a pinned tree digest, flattens lookup precedence deterministically, and writes an immutable manifest plus content-addressed objects. Build and test it explicitly with `cargo test --manifest-path tools/texlive-wasm-publish/Cargo.toml`; it must not join the root workspace or make ordinary tests scan a TeX Live installation.
Its manifest model and canonical serialization come from the workspace
`umber-distribution` crate; schema changes must keep the shared Rust/JavaScript
fixtures green.
Production snapshots use `scripts/build-texlive-snapshot.sh`, which scans the
full runtime-requestable TeX Live tree, derives bounded package hints from the
pinned `texlive.tlpdb`, and enforces inventory floors. The smaller
`build-wasm-latex-bundle.sh` remains a focused LaTeX seed/fixture builder and
must not be used for production publication.
The publisher's explicit `html` profile instead builds a new schema-4
distribution from selected format closures, runtime TeX/TFM objects, and an
exact curated WOFF2/mapping/license catalog. It does not mutate or filter the
schema-3 production snapshot in place.

`tools/parity-harness` is the shared Rust library and opt-in compatibility CLI for end-to-end DVI conformance. Oracle-presence-conditional Story, Gentle, TRIP, and e-TRIP tests use its default library for final artifact comparison against gitignored, locally generated `tests/corpus/e2e` DVI files, without compiling live reference execution. Its fixture path stages manifest inputs and calls an in-process Umber runner supplied by the Cargo test; it never launches the Umber binary. The `reference-tools` feature enables the CLI and live-reference paths used by `scripts/regen-fixtures.sh`; the explicit `--write-reference-fixture` path verifies manifest-pinned reference hashes and writes local oracles. Comparison uses `test-support` to normalize only DVI preamble comments, requires byte-identical final DVI, and writes automatic bundles under `target/conformance-triage/` or the CLI-selected triage directory.

`tools/parity-harness/src/trip_triage.rs` owns the compact TRIP-specific v1
artifact. It compares canonical `tex-oracle` event streams before transcript,
log, and preamble-normalized DVI channels, writes no copied outputs, and keeps
the deterministic report bounded.

`tools/profile-analyzer` is the read-only Samply/Firefox processed-profile CLI.
It reconstructs columnar sample stacks, consumes Samply presymbolication
sidecars including inline frames, and reports self/inclusive, subtree, and
runtime-caller attribution for persistent engine profiles.

`tools/tex-command-stream` is the offline, test-only canonical command-stream
comparison runner. It replays TeX82 command fixture inputs through the
instrumented command boundary, translates the owned observer records into the
portable `tex-oracle` schema, and reports a ranked worklist of up to
`--max-divergences` ordered divergences (stream mismatches and contained
replay failures alike). It never invokes a reference engine or joins the
production engine dependency graph.

Comparison is a two-tier keyed sequence alignment, not an index-parallel
scan: `src/compare.rs` splits each event into an identity key and a payload,
reports a payload-only difference once without desynchronizing, and repairs a
key mismatch with a bounded wavefront search (`--realign-window`,
`--realign-confirm`) followed by a structural anchor fallback that rejoins at
the least-total-skip shared anchor inside `--anchor-scan`, stopping a
fixture's comparison outright rather than guessing when neither confirms.
Each entry names its repair and the cascade it suppressed. See "Stream
alignment" in `docs/testing_infrastructure.md`.

`src/group.rs` then collapses exact recurrences of one root site into a single
entry, and `src/report.rs` renders the worklist. Grouping is presentation
only: the comparison, the entry order, and the divergence count are identical
with and without it, `--ungrouped` prints the one-entry-per-divergence
worklist, and the header prints both totals with labels saying which is
comparable against historical figures. Two divergences group only when they
are equal after erasing every source position and nothing else, through an
exhaustive match over the event schema, so a new schema variant fails to
compile rather than silently changing what merges. The report also prints the
bounds it applied: a fixture whose `--max-divergences` budget was reached, and
a document trace that was never generated. Either bound also makes the run
`PARTIAL` (exit `2`), separate from `CLEAN` (`0`), `DIVERGED` (`1`), and a run
that could not be performed (`3`), so a caller reading only the exit status
cannot take an uncompared fixture for a converged one; the report is printed
even when clean and closes with a matching `VERDICT:` line. See "Grouped
worklist and run accounting" in `docs/testing_infrastructure.md`.

The runner owns no canonical vocabulary of its own. Every name it puts in an
event -- catcodes, command names, glue orders, scanner statuses, token
spellings -- comes from `tex_command::canonical_names`, and the command name
comes from the producer's record rather than being re-derived from the
spelling. Keeping a second table here is how `umber2-johp.141` found a
transport-side catcode table that had drifted from tex.web's §207 names in
seven places while silently masking an engine divergence; see "Canonical
Observation Vocabulary" in `crates/tex-command/AGENTS.md`.

It replays two registries: the committed, always-present fixtures under
`tests/corpus/command/tex82`, then the full-document traces (plain bootstrap,
Story, Gentle) under `tests/corpus/command/tex82-documents`. The document tier
is generated on demand by `scripts/build-tex82-document-traces.sh` and
gitignored -- those traces run 17-156 MB each -- with its identity pinned in
`tests/tex82-document-trace-manifest.txt`; an ungenerated document is skipped
with a stderr notice rather than failing. Document replay registers the staged
TFM set through `CommandHostCapabilities::register_font` before its first step,
because canonical font resolution fails rather than suspends. See
"Differential Tracer" in `docs/testing_infrastructure.md`.

`scripts/fetch-conformance-inputs.sh` acquires the external hyphenation and
Computer Modern font inputs and fetches and verifies the pinned official Knuth
TeX82 TRIP and e-TeX V2 e-TRIP materials. Cargo integration tests execute the
two-phase format workflow directly in Rust and reuse the pinned `trip.tfm` for
e-TRIP. Fixture regeneration independently runs the two-phase reference
workload with pdfTeX.
