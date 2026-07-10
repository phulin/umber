# Deliberate TeX82 Divergences

## `\dump` does not serialize format files

Umber recognizes `\dump` as TeX82's stop command and runs the same final page
cleanup as `\end`, but it emits one warning and writes no format file. This
keeps INITEX-style macro-package loading usable while format serialization and
loading remain unimplemented.

Reference: `tex.web`'s `primitive("dump", stop, 1)`, `final_cleanup`, and
`store_fmt_file` in the format-file section.
