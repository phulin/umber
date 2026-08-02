# umber2-johp.455 — insertion-skip error context

Authority: TeX82 §§82, 90, 1009, and 1100.

Closing an outer-vertical insertion invokes the page builder before main
control fetches another command. If the insertion class's correction glue has
non-normal finite shrink, §1009 calls `error` at that boundary. Its §82 input
display is therefore the still-live closing insertion command, and it precedes
§90's transcript help.

The canonical command core owns that live input stack and passes its rendered
error context into the page builder. Page-builder diagnostics consume that
context when present; callers outside the canonical command path retain the
Universe input-summary fallback.
