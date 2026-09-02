# umber2-sdpz.246: preserve VF first-use resource order

The pinned TeX Live 2026 reference for row `2606.00086` remains
`/home/phulin/umber/target/pdf-parity-reference-2606.00086/run/root_first_submission.pdf`,
SHA-256
`e430c3db54077bde9882b386f10c507c8ee6979707ba3479c3ff4ed50632a981`.
No reference was regenerated.

## First divergence and fix

After scalable VF-leaf sharing, page 1 selected the correct Times programs but
named them F130, F128, and F129 instead of pdfTeX's F124, F125, and F126. The
content stream selected the same incorrect names. Umber had collected virtual
root identities and codes into sorted maps after completing the whole job, then
allocated leaves after the final font resource. That erased pdfTeX's temporal
first-use order.

pdftex.web section 32e loads a virtual font's local definitions when that font
is first output, and `pdf_set_font` names the real leaf with its internal font
number. Each committed PDF page now captures the highest one-based engine font
number live at its shipout. Detached VF discovery replays pages, text runs, and
codes in original order, deduplicating only after first occurrence, and starts
new leaf numbers immediately after that page watermark. The synthetic
two-size VF control proves that a defined-but-not-painted root still contributes
to the watermark and that the default and explicitly selected leaves receive
consecutive first-use resources while scalable aliases continue sharing the
first dictionary.

## Row evidence

The authenticated fresh-source run used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, offline mode, 500,000,000 expansion fuel, 10,000,000
execution steps, a 120-second wall guard, two-second termination grace, and the
authorized 2,048-MiB aggregate-RSS ceiling. The resulting PDF SHA-256 is
`754b31f78ba608223e3ddb5c0633bd6f96b9013bbabaf9b8c35b3a41e4b81a3e`;
its normalized projection is
`39d7d849cb57f954beebaaefbc5f3f8728ed64f03d79c30c54a7820fe5cc69b9`.
The AUX remains byte-exact at
`013c530c5182267623c8a92f1e5b914751f4d64c3022ed3f3f6a1df693a0a0dd`.

Page 1 now has the exact resource names and BaseFonts F124
`NimbusRomNo9L-Medi`, F125 `NimbusRomNo9L-Regu`, and F126
`NimbusRomNo9L-MediItal`. Its normalized content is exact when the resource
dictionary line is excluded. The first remaining difference is the three Times
subset encoding/program payloads inside that resource dictionary; it is the
sole successor `umber2-sdpz.247`.
