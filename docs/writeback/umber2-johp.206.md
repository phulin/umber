# umber2-johp.206 — prefixed-command filler and recovery

TeX82 §404 expands calls and skips both blank and `\relax` commands before
returning a substantive command. TeX82 §1211 uses that helper after every
accumulated `\global`, `\long`, or `\outer` prefix; the similar §406 helper,
which skips blanks only, remains correct for §1045's `\ignorespaces`.

Canonical main control now calls the command-owned §404 helper only inside
`prefixed_command`. Prefix bits therefore survive spaces, `\relax`, and
expanded macro calls. When the returned command is at or below
`max_non_prefixed_command`, §1212 emits its exact diagnostic and help, discards
the accumulated prefixes, and replays the substantive command once through
`back_error`.

Focused units cover all three TeX82 prefix bits, repeated prefixes, group
scope, exact recovery text and token replay, and `\afterassignment` ordering.
The committed `main-control/prefix-collection` semantic minifixture covers the
same §404/§1211 boundary and §1212 recovery without adding full-document
automated tracing.
