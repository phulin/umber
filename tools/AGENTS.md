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

`tools/profile-analyzer` is the read-only Samply/Firefox processed-profile CLI.
It reconstructs columnar sample stacks, consumes Samply presymbolication
sidecars including inline frames, and reports self/inclusive, subtree, and
runtime-caller attribution for persistent engine profiles.

`scripts/fetch-conformance-inputs.sh` acquires the external hyphenation and
Computer Modern font inputs and fetches and verifies the pinned official Knuth
TeX82 TRIP and e-TeX V2 e-TRIP materials. Cargo integration tests execute the
two-phase format workflow directly in Rust and reuse the pinned `trip.tfm` for
e-TRIP. Fixture regeneration independently runs the two-phase reference
workload with pdfTeX.
