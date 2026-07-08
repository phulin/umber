# Tools Guidance

`tools/refexec` is a host-side utility crate for parity harnesses: it runs the machine reference TeX (`pdftex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default it locates `pdftex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point tests or the CLI at a different reference binary, such as a specific TeX Live installation.

`refexec` also wraps `tftopl` for font metric parity tests. By default it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point tests at a specific TeX installation.
