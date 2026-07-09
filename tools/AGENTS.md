# Tools Guidance

`tools/refexec` is a host-side workspace utility crate for regeneration tooling and committed DVI fixture comparison: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default the tool locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point fixture regeneration at a different reference binary, such as a specific TeX Live installation. Its DVI comparison path accepts extra copied inputs for pinned local TFMs and normalizes only the preamble comment payload before byte comparison.

`tools/fixturegen` is the script-owned fixture regeneration tool used by `scripts/regen-fixtures.sh` for text/native fixtures and the explicit live font check. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/fixturegen/Cargo.toml`. It may invoke `refexec`, `umber`, and `tftopl`, but cargo tests must not build or run it.

`refexec` also wraps `tftopl` for the font metric check owned by `tools/fixturegen`. When running that tier, it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point regeneration at a specific TeX installation.

`tools/corpus-sync` is the script-owned external document acquisition tool used by `scripts/parity.sh`. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/corpus-sync/Cargo.toml`. It reads the line-oriented `tests/corpus-manifest.txt`, fetches exact bytes into gitignored `third_party/corpus/`, verifies SHA-256, and treats cached hash matches as a no-op so offline runs can succeed after the corpus has been populated. Do not normalize line endings or commit fetched corpus documents; licensing determinations live in the manifest notes.

`tools/parity-harness` is the script-owned end-to-end corpus parity runner used by `scripts/parity.sh e2e`. It runs the acquired manifest documents through reference TeX via `refexec` and through the `umber` binary, verifies the manifest-pinned normalized reference DVI hash, byte-compares normalized DVI output, and writes automatic bundles under `target/parity-triage/` for reference drift, Umber failures, or DVI mismatches. Keep this long-running live-reference workflow out of default cargo tests; its fast synthetic `--self-test` exists only to validate bundle summary/disassembly behavior.

`scripts/trip.sh` owns the official Knuth TeX82 TRIP conformance tier directly
with shell plus ambient TeXware tools; it does not currently have a Rust tool
crate under `tools/`. Keep it outside default cargo tests. It fetches the
pinned CTAN files in `tests/trip-manifest.txt`, requires PLtoTF/TFtoPL/DVItype,
and requires `UMBER_TRIP_INITEX` to point at Knuth's special TRIP INITEX build
for a passing reference phase.
