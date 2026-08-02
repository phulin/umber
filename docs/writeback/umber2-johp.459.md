# umber2-johp.459 — feasible-break short-display range

Authority: TeX82 §§175 and 851.

When tracing a newly feasible paragraph break, TeX temporarily terminates the
paragraph list at `cur_p` and passes the resulting list to `short_display`.
The displayed range therefore includes the breakpoint node itself. This is
observable for a glue breakpoint because §175 renders its nonzero glue as a
trailing space, and for a discretionary because its pre-break and post-break
lists are rendered.

The pure line-breaking kernel retains scalar width accounting separately: its
line width still ends before a discarded breakpoint glue. Only the detached
diagnostic node range includes `cur_p`; layout and break selection are
unchanged.
