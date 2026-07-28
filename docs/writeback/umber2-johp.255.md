# umber2-johp.255 — restricted input-stream selectors

TeX82 §435's `scan_four_bit_int` is the common selector scan used by §1225's
`\read` and §§1272-1275's `\openin` and `\closein`; e-TeX's `\readline` uses
the same restricted input-stream selector. The ordinary integer result is
observed first. Values outside 0 through 15 then produce `Bad number`, retain
the raw value for `int_error`, and replace the value consumed by the request
with zero before stream state or the read target is committed.

The structured scanner now carries the recovered selector plus the raw value
and recovery bit across the typed apply seam. Focused units cover -1, 15, 16,
and 1000000 across all four consumers; the committed semantic microfixture
checks the recovery diagnostics and subsequent command replay.
