# umber2-johp.39 — current-group right-brace dispatch

Authority: TeX82 `tex.web` §§274, 1016, and 1068 (`new_save_level`, output
routine entry, and `handle_right_brace`), confirmed unchanged by pdfTeX 1.40.27
`pdftex.web` §§274, 1016, and 1068.

`handle_right_brace` selects its action from the live `cur_group`; it does not
infer a simple group from an ancestor depth counter. Therefore a right brace
closing a nested `vbox_group` inside a still-open simple group packages that
vbox first. The output routine closes only when the live group is
`output_group`, after its nested boxes and ordinary groups have retired.
