% pdfTeX command-state, enquiry, and PDF-effect observation.
% Applied after extension-instrumentation.ch so it can use those schema helpers.

@x [26] Keep the host-timed result out of deterministic semantic values.
@<Fix the reference count, if any, and negate |cur_val| if |negative|@>;
umber_trace_scanner(3,cur_val_level);
exit:end;
@y
@<Fix the reference count, if any, and negate |cur_val| if |negative|@>;
if m=16 then {canonical pdfTeX 1.40.29 |elapsed_time_code|}
  begin
  umber_trace_pdf_enquiry(3,cur_val);
  umber_pdf_timer_enquiry:=true;
  end
else umber_trace_scanner(3,cur_val_level);
exit:end;
@z

@x [53a] Observe object-independent pdfTeX integer enquiries.
  pdf_last_x_pos_code:  cur_val := pdf_last_x_pos;
  pdf_last_y_pos_code:  cur_val := pdf_last_y_pos;
  pdf_retval_code:      cur_val := pdf_retval;
  pdf_last_ximage_colordepth_code: cur_val := pdf_last_ximage_colordepth;
  elapsed_time_code: cur_val := get_microinterval;
  random_seed_code:  cur_val := random_seed;
  pdf_shell_escape_code:
    begin
    if shellenabledp then begin
      if restrictedshell then cur_val :=2
      else cur_val := 1;
    end
    else cur_val := 0;
    end;
@y
  pdf_last_x_pos_code: begin cur_val := pdf_last_x_pos;
    umber_trace_pdf_enquiry(0,cur_val); end;
  pdf_last_y_pos_code: begin cur_val := pdf_last_y_pos;
    umber_trace_pdf_enquiry(1,cur_val); end;
  pdf_retval_code: begin cur_val := pdf_retval;
    umber_trace_pdf_enquiry(2,cur_val); end;
  pdf_last_ximage_colordepth_code: cur_val := pdf_last_ximage_colordepth;
  elapsed_time_code: cur_val := get_microinterval;
  random_seed_code: begin cur_val := random_seed;
    umber_trace_pdf_enquiry(5,cur_val); end;
  pdf_shell_escape_code:
    begin
    if shellenabledp then begin
      if restrictedshell then cur_val :=2
      else cur_val := 1;
    end
    else cur_val := 0;
    umber_trace_pdf_enquiry(4,cur_val);
    end;
@z

@x [26] Suppress the enclosing integer result for the volatile timer sample.
if negative then negate(cur_val);
umber_trace_scanner(0,0);
end;
@y
if negative then negate(cur_val);
if umber_pdf_timer_enquiry then umber_pdf_timer_enquiry:=false
else umber_trace_scanner(0,0);
end;
@z

@x [27] Observe font name and size without PDF object identity.
pdf_font_name_code, pdf_font_objnum_code, pdf_font_size_code: begin
    scan_font_ident;
    if cur_val = null_font then
        pdf_error("font", "invalid font identifier");
    if c <> pdf_font_size_code then begin
        pdf_check_vf_cur_val;
        if not font_used[cur_val] then
            pdf_init_font_cur_val;
    end;
end;
@y
pdf_font_name_code, pdf_font_objnum_code, pdf_font_size_code: begin
    scan_font_ident;
    if cur_val = null_font then
        pdf_error("font", "invalid font identifier");
    if c <> pdf_font_size_code then begin
        pdf_check_vf_cur_val;
        if not font_used[cur_val] then
            pdf_init_font_cur_val;
    end;
    if c=pdf_font_name_code then
      begin
      umber_trace_pdf_enquiry(6,cur_val);
      umber_pdf_object_tokens:=true;
      end
    else if c=pdf_font_size_code then
      umber_trace_pdf_enquiry(7,font_size[cur_val]);
end;
@z

@x [27] Observe committed margin-kern enquiry results.
left_margin_kern_code: begin
    p := list_ptr(p);
    while (p <> null) and
          (cp_skipable(p) or
           ((not is_char_node(p)) and (type(p) = glue_node) and (subtype(p) = left_skip_code + 1)))
    do
       p := link(p);
    if (p <> null) and (not is_char_node(p)) and
       (type(p) = margin_kern_node) and (subtype(p) = left_side) then
        print_scaled(width(p))
    else
        print("0");
    print("pt");
end;
right_margin_kern_code: begin
    q := list_ptr(p);
    p := prev_rightmost(q, null);
    while (p <> null) and
          (cp_skipable(p) or
           ((not is_char_node(p)) and (type(p) = glue_node) and (subtype(p) = right_skip_code + 1)))
    do
        p := prev_rightmost(q, p);
    if (p <> null) and (not is_char_node(p)) and
       (type(p) = margin_kern_node) and (subtype(p) = right_side) then
        print_scaled(width(p))
    else
        print("0");
    print("pt");
end;
@y
left_margin_kern_code: begin
    p := list_ptr(p);
    while (p <> null) and
          (cp_skipable(p) or
           ((not is_char_node(p)) and (type(p) = glue_node) and (subtype(p) = left_skip_code + 1)))
    do
       p := link(p);
    if (p <> null) and (not is_char_node(p)) and
       (type(p) = margin_kern_node) and (subtype(p) = left_side) then
      begin umber_trace_pdf_enquiry(8,width(p)); print_scaled(width(p)); end
    else begin umber_trace_pdf_enquiry(8,0); print("0"); end;
    print("pt");
end;
right_margin_kern_code: begin
    q := list_ptr(p);
    p := prev_rightmost(q, null);
    while (p <> null) and
          (cp_skipable(p) or
           ((not is_char_node(p)) and (type(p) = glue_node) and (subtype(p) = right_skip_code + 1)))
    do
        p := prev_rightmost(q, p);
    if (p <> null) and (not is_char_node(p)) and
       (type(p) = margin_kern_node) and (subtype(p) = right_side) then
      begin umber_trace_pdf_enquiry(9,width(p)); print_scaled(width(p)); end
    else begin umber_trace_pdf_enquiry(9,0); print("0"); end;
    print("pt");
end;
@z

@x [27] Observe committed insertion-height enquiry results.
pdf_insert_ht_code: begin
    i := qi(cur_val);
    p := page_ins_head;
    while i >= subtype(link(p)) do
        p := link(p);
    if subtype(p) = i then
        print_scaled(height(p))
    else
        print("0");
    print("pt");
end;
@y
pdf_insert_ht_code: begin
    i := qi(cur_val);
    p := page_ins_head;
    while i >= subtype(link(p)) do
        p := link(p);
    if subtype(p) = i then
      begin umber_trace_pdf_enquiry(10,height(p)); print_scaled(height(p)); end
    else begin umber_trace_pdf_enquiry(10,0); print("0"); end;
    print("pt");
end;
@z

@x [40] Observe a committed PDF page without backend object identity.
if type(p)=vlist_node then pdf_vlist_out@+else pdf_hlist_out;
if shipping_page then
    incr(total_pages);
cur_s:=-1;
@<Finish shipping@>;
done:
@y
if type(p)=vlist_node then pdf_vlist_out@+else pdf_hlist_out;
if shipping_page then
    incr(total_pages);
cur_s:=-1;
@<Finish shipping@>;
if shipping_page then umber_trace_effect(4,0,2,total_pages);
done:
@z

@x [53a] Observe pdfTeX font code-table commits.
assign_font_int: begin n:=cur_chr; scan_font_ident; f:=cur_val;
  if n = no_lig_code then set_no_ligatures(f)
  else if n < lp_code_base then begin
    scan_optional_equals; scan_int;
    if n=0 then hyphen_char[f]:=cur_val@+else skew_char[f]:=cur_val;
  end
  else begin
    scan_char_num; p := cur_val;
    scan_optional_equals; scan_int;
    case n of
    lp_code_base: set_lp_code(f, p, cur_val);
    rp_code_base: set_rp_code(f, p, cur_val);
    ef_code_base: set_ef_code(f, p, cur_val);
    tag_code:     set_tag_code(f, p, cur_val);
    kn_bs_code_base: set_kn_bs_code(f, p, cur_val);
    st_bs_code_base: set_st_bs_code(f, p, cur_val);
    sh_bs_code_base: set_sh_bs_code(f, p, cur_val);
    kn_bc_code_base: set_kn_bc_code(f, p, cur_val);
    kn_ac_code_base: set_kn_ac_code(f, p, cur_val);
    end;
  end;
end;
@y
assign_font_int: begin n:=cur_chr; scan_font_ident; f:=cur_val;
  if n = no_lig_code then
    begin set_no_ligatures(f); umber_trace_pdf_font_code(n,f,-1,1); end
  else if n < lp_code_base then begin
    scan_optional_equals; scan_int;
    if n=0 then hyphen_char[f]:=cur_val@+else skew_char[f]:=cur_val;
  end
  else begin
    scan_char_num; p := cur_val;
    scan_optional_equals; scan_int;
    case n of
    lp_code_base: set_lp_code(f, p, cur_val);
    rp_code_base: set_rp_code(f, p, cur_val);
    ef_code_base: set_ef_code(f, p, cur_val);
    tag_code:     set_tag_code(f, p, cur_val);
    kn_bs_code_base: set_kn_bs_code(f, p, cur_val);
    st_bs_code_base: set_st_bs_code(f, p, cur_val);
    sh_bs_code_base: set_sh_bs_code(f, p, cur_val);
    kn_bc_code_base: set_kn_bc_code(f, p, cur_val);
    kn_ac_code_base: set_kn_ac_code(f, p, cur_val);
    end;
    umber_trace_pdf_font_code(n,f,p,cur_val);
  end;
end;
@z

@x [53a] Observe deterministic random-state replacement.
  random_seed := cur_val;
  init_randoms(random_seed);
end
@y
  random_seed := cur_val;
  init_randoms(random_seed);
  umber_trace_pdf_state(1,random_seed);
end
@z

@x [53a] Observe timer reset without recording host clock identity.
begin
  seconds_and_micros(epochseconds,microseconds);
end
@y
begin
  seconds_and_micros(epochseconds,microseconds);
  umber_trace_pdf_state(0,0);
end
@z

@x [53a] Observe the ordered interword-space-on whatsit.
    new_whatsit(pdf_interword_space_on_node, small_node_size);
end
@y
    new_whatsit(pdf_interword_space_on_node, small_node_size);
    umber_trace_pdf_state(2,0);
end
@z

@x [53a] Observe the ordered interword-space-off whatsit.
    new_whatsit(pdf_interword_space_off_node, small_node_size);
end
@y
    new_whatsit(pdf_interword_space_off_node, small_node_size);
    umber_trace_pdf_state(3,0);
end
@z

@x [53a] Observe the ordered fake-space whatsit.
    new_whatsit(pdf_fake_space_node, small_node_size);
end
@y
    new_whatsit(pdf_fake_space_node, small_node_size);
    umber_trace_pdf_state(4,0);
end
@z

@x [53a] Observe the ordered running-link-off whatsit.
    new_whatsit(pdf_running_link_off_node, small_node_size);
end
@y
    new_whatsit(pdf_running_link_off_node, small_node_size);
    umber_trace_pdf_state(5,0);
end
@z

@x [53a] Observe the ordered running-link-on whatsit.
    new_whatsit(pdf_running_link_on_node, small_node_size);
end
@y
    new_whatsit(pdf_running_link_on_node, small_node_size);
    umber_trace_pdf_state(6,0);
end
@z

@x [53a] Observe the expanded global fallback-space font selection.
    pdf_space_font_name := tokens_to_string(def_ref);
    delete_token_ref(def_ref);
end
@y
    pdf_space_font_name := tokens_to_string(def_ref);
    delete_token_ref(def_ref);
    umber_trace_pdf_space_font(pdf_space_font_name);
end
@z
