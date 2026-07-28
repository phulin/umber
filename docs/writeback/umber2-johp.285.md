# umber2-johp.285 — committed geometry projection

Authority: TeX82 `tex.web` §§633, 668, and 664 (`hpack`, `vpackage`, and
`ship_out`).

The routine differential gate now consumes the separately pinned schema-v2
geometry projection alongside the immutable schema-v1 command fixtures. It
replays only the committed font-independent microfixture, enables canonical
geometry observation, and compares finalized hpack and vpack dimensions and
shipout page totals as signed scaled points. The fixture covers explicit
packaging, paragraph line packing, explicit shipment, and end-of-job
page-builder shipment.

Geometry remains a detached projection: ordinary command events are neither
removed from schema v1 nor added to the geometry expectation. Controlled box
and page-total mutations report a local `geometry_mismatch` carrying both
records. Gentle and all other full documents remain manual, generated
diagnostics and are not part of the native suite.
