# umber2-johp.119 — canonical e-TeX and pdfTeX trace strings

Authority: TeX82 `tex.web` §§355 and 356 for in-place `^^` reduction and
control-sequence lookahead; the detached trace transport's canonical JSON
round-trip contract for string encoding.

The e-TeX 2.6 and pdfTeX 1.40.29 observers now emit JSON strings with Web2C's
`xchr` table, JSON's five short control escapes, `\u00xx` only for the
remaining C0 controls, literal DEL, and two-byte UTF-8 for character codes
128–255. Each shared transition program directly exercises all four encoding
classes, and its executable matrix requires the exact emitted bytes.

Both observers also preserve immutable source columns across §355 buffer
collapses. The per-line accumulated shift is stacked with file input state and
reset for each new line; the most recent collapse position distinguishes a
delivered reduced spelling from §356 lookahead strictly to the command's
right.
