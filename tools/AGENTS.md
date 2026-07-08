# Tools Guidance

`tools/refexec` is a host-side utility crate for parity harnesses: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default it locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point tests or the CLI at a different reference binary, such as a specific TeX Live installation. Its DVI comparison path accepts extra copied inputs for pinned local TFMs and normalizes only the preamble comment payload before byte comparison.

`refexec` also wraps `tftopl` for font metric parity tests. By default it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point tests at a specific TeX installation.
