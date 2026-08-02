# umber2-johp.465 — output-resume error context

Authority: TeX82 `tex.web` §§82, 1009, and 1026.

After an output routine ends, §1026 resumes `build_page` before returning to
main control. The input stack below the exhausted output token list is still
live, so a synchronous §1009 insertion-correction error must use §82's context
from the command that triggered output.

The canonical output-resume boundary already receives that rendered context
for post-output `\box255` recovery. It now forwards the same context into the
resumed page builder instead of falling back to a stale input summary. A
focused regression stages an insertion independently of command scanning and
proves message, supplied live context, and help ordering.

Guarded format-loaded TRIP changes the actual log SHA-256 from
`737e04ff4cbad0630c060fcea27177caed1801156edb19657d04840bbe1048d7` to
`06ca3424345b4edc6168002b5fb21150f6063ab0d65795eb1237a40d7e774bb3` while
retaining exact normalized DVI and all 22 command events. The remaining
conditional-result trace front is tracked by `umber2-johp.470`.
