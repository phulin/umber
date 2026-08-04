# umber2-johp.744 — discretionary diagnostic escape projection

Authority: TeX82 `tex.web` §§63 and 187.

Detailed node-list display names a discretionary with `print_esc`. Its header
therefore observes the live `\escapechar`, including while recursively showing
a post-break list. The node dumper now routes this header through the shared
escape-name renderer instead of embedding a backslash.

A focused node-dump regression sets `\escapechar` to `|` and verifies the
resulting `|discretionary` header. Guarded format-loaded TRIP advances the
first normalized log mismatch from byte 171781 to byte 171852. All 22 command
events and the normalized DVI remain byte-exact.
