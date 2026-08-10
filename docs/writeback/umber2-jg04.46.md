# umber2-jg04.46: merged e-TeX string-pool coordinates

The exhaustive canonical tracer is clean. Once compatibility TRIP became
exact, official two-phase e-TRIP first differed in the final string-capacity
row: the pinned oracle reported `18` used strings out of `13506`, while Umber
reported the same usage out of `13504`. Command projections and normalized DVI
were exact.

Pinned Web2C stops at `init_prim` measured the complete engine-owned pool image
before user input. TeX82 held `str_ptr=1349` and `pool_ptr=24541`; e-TeX 2.6
held `1468/26162`. Thus the merged image is exactly 119 strings and 1621 bytes
beyond TeX82. TeX82 §§47/50 own the static string-pool lifecycle, while e-TeX
[1.2] supplies the merged program literals and version vocabulary.

Umber's typed registry previously landed 121 strings and 1653 bytes beyond its
TeX82 coordinate. The e-TeX profile had compensated for two strings and 32
spelling bytes that the typed primitive installation already owned. The
generic profile now lands on the measured upstream coordinates. Focused
controls pin both deltas, repeated profile selection, and repeated installation
as no-ops. Persisted pool profile version 9 and prepared-format producer
contract 12 force clean reconstruction instead of accepting an older image.

Official e-TRIP advances to an independent character-capacity difference:
`13644` expected and `13643` actual. That observed-only successor is
`umber2-jg04.48`; the preceding string row is exact and no comparison or
normalization changed.
