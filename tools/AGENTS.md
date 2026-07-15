# Tools Guidance

`tools/refexec` is a host-side workspace utility crate for regeneration tooling and committed DVI fixture comparison: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default the tool locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point fixture regeneration at a different reference binary, such as a specific TeX Live installation. Its DVI comparison path accepts extra copied inputs for pinned local TFMs and normalizes only the preamble comment payload before byte comparison.

`tools/fixturegen` is the script-owned fixture regeneration tool used by `scripts/regen-fixtures.sh` for text/native fixtures, pinned pdfTeX/Poppler PDF parity fixtures, and the explicit live font check. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/fixturegen/Cargo.toml`. It may invoke `refexec`, `umber`, `pdftex`, `pdftoppm`, and `tftopl`, but cargo tests must not build or run it.

`refexec` also wraps `tftopl` for the font metric check owned by `tools/fixturegen`. When running that tier, it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point regeneration at a specific TeX installation.

`tools/corpus-sync` is the external document acquisition tool used by `scripts/setup-conformance-tests.sh`. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/corpus-sync/Cargo.toml`. It reads the line-oriented `tests/corpus-manifest.txt`, fetches exact support inputs and runnable documents into gitignored `third_party/corpus/`, verifies SHA-256, and treats cached hash matches as a no-op. Once setup is complete, conformance tests consume only local inputs and require no network access. Do not normalize line endings or commit fetched corpus files; licensing determinations live in the manifest notes.

`tools/texlive-wasm-publish` is a standalone release tool for browser TeX Live assets. It verifies every configured TEXMF root against a pinned tree digest, flattens lookup precedence deterministically, and writes an immutable manifest plus content-addressed objects. Build and test it explicitly with `cargo test --manifest-path tools/texlive-wasm-publish/Cargo.toml`; it must not join the root workspace or make ordinary tests scan a TeX Live installation.

`tools/parity-harness` is the shared Rust library and compatibility CLI for end-to-end DVI conformance. Oracle-presence-conditional Story, Gentle, TRIP, and e-TRIP tests use it for final artifact comparison against gitignored, locally generated `tests/corpus/e2e` DVI files. Its fixture path stages manifest inputs and calls an in-process Umber runner supplied by the Cargo test; it never launches the Umber binary. The explicit `--write-reference-fixture` CLI path stages manifest-selected documents under live reference TeX for `scripts/regen-fixtures.sh`, verifies manifest-pinned reference hashes, and writes local oracles. Comparison normalizes only DVI preamble comments, requires byte-identical final DVI, and writes automatic bundles under `target/conformance-triage/` or the CLI-selected triage directory.

`tools/profile-analyzer` is the read-only Samply/Firefox processed-profile CLI.
It reconstructs columnar sample stacks, consumes Samply presymbolication
sidecars including inline frames, and reports self/inclusive, subtree, and
runtime-caller attribution for persistent engine profiles.

`scripts/trip.sh` owns specialized official Knuth TeX82 TRIP and e-TeX V2
e-TRIP preparation and standalone compatibility orchestration with shell plus
ambient TeXware tools.
Cargo integration tests do not invoke this script; they execute the same
two-phase format workflow directly in Rust. The script fetches the pinned CTAN
files in `tests/trip-manifest.txt`, reuses the pinned `trip.tfm`
directly for e-TRIP, requires DVItype only for the
standalone reference phase, and requires `UMBER_TRIP_INITEX` to point at
Knuth's special TRIP INITEX build for a passing reference phase.
