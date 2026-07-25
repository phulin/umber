# umber2-johp.8.3.7 — macro-definition scanner-status ordering

Authority: TeX82 `tex.web` §§391–394 (`macro_call` and its parameter matcher),
§25 (`back_input`), and §476 (`scan_toks`).

For a non-`\long` macro argument containing `\par`, §394 reaches its runaway
recovery through `back_error` while `scanner_status=matching`. The paragraph
token is therefore backed up before `macro_call` restores the enclosing
definition scanner status. The still-live expanded `scan_toks` collector then
rescans that backed-up `\par` as replacement text. The command core preserves
that ordering; fixture observations remain unchanged.
