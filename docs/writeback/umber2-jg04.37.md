# umber2-jg04.37: Fresh-cache TRIP format graph panic

The exhaustive canonical command tracer is clean. A fresh isolated XDG cache
reproduced the compatibility TRIP panic while TeX82 §638 computed live memory
usage for a shipped box. The earliest invariant was in frozen node-graph
capture: `FormatNode` serialized both semantic children and detached physical
diagnostic children, but discovery traversed only semantic children. Canonical
key remapping consequently had no captured target for a diagnostic edge.

Format and detached shipout-node capture now use the node arena's existing
physical traversal. This preserves the distinct semantic topology while making
the serialized DTO graph self-contained. TeX82 §§115/162 define the physical
discretionary replacement representation, §182 walks it for diagnostics, and
§638 can observe the containing graph during shipout memory accounting. The
focused regression round-trips a box with distinct semantic and physical child
lists through the frozen format encoder and decoder.

At commit `89b73e7ae`, the focused `tex-state` suite, official e-TRIP, routine
workspace tests, and all four repository checks pass. Fresh-cache TeX82 TRIP no
longer panics; command events, geometry, and normalized DVI are exact. Its first
remaining difference is a log-only §283 restore trace caused by the independent
page-builder §993 `box_error` mutation boundary, filed observed-only as
`umber2-jg04.38`. The exact compatibility gate and therefore this issue remain
open pending that child.
