# Tools Guidance

`tools/refexec` is a host-side utility crate for regeneration tooling: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default the tool locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point fixture regeneration at a different reference binary, such as a specific TeX Live installation. Its DVI comparison path accepts extra copied inputs for pinned local TFMs and normalizes only the preamble comment payload before byte comparison.

`tools/fixturegen` is the script-owned fixture regeneration tool used by `scripts/regen-fixtures.sh` for text/native fixtures and the explicit live font check. It may invoke `refexec`, `umber`, and `tftopl`, but cargo tests must not.

`refexec` also wraps `tftopl` for the font metric check owned by `tools/fixturegen`. When running that tier, it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point regeneration at a specific TeX installation.
