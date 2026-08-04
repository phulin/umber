% pdfTeX 1.40.29 command-extension observation layered after instrumentation.ch.
% The transport and schema helpers are owned by the preceding change file.

@x [18] Track the one volatile enquiry whose numeric result is host-timed.
@!umber_alignment_depth:integer;
@!umber_mutation_command:boolean;
@y
@!umber_alignment_depth:integer;
@!umber_mutation_command:boolean;
@!umber_pdf_timer_enquiry:boolean;
@!umber_pdf_object_tokens:boolean;
@z

@x [18] Exclude PDF allocation identity from command delivery.
procedure umber_trace_command(@!expanded:boolean);
begin
if not umber_trace_opened then return;
@y
procedure umber_trace_command(@!expanded:boolean);
begin
if (not umber_trace_opened)or(umber_pdf_object_tokens) then return;
@z

@x [18] Retire allocation-token suppression with its inserted input.
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_token(@!t:halfword);
@y
write_ln(umber_trace_file,'}}}');
if (transition=1)and umber_pdf_object_tokens then
  umber_pdf_object_tokens:=false;
end;

procedure umber_trace_token(@!t:halfword);
@z

@x [18] Exclude PDF allocation identity from recovery-token events.
procedure umber_trace_recovery(@!kind:integer;@!t:halfword);
begin
if not umber_trace_opened then return;
@y
procedure umber_trace_recovery(@!kind:integer;@!t:halfword);
begin
if (not umber_trace_opened)or(umber_pdf_object_tokens) then return;
@z

@x [18] Name pdfTeX parameter slots semantically and add extension helpers.
procedure umber_trace_named_slot(@!family:integer;@!slot:integer);
begin
write(umber_trace_file,'"');
if family=9 then
  case slot of
  tracing_assigns_code:write(umber_trace_file,'tracingassigns');
  tracing_groups_code:write(umber_trace_file,'tracinggroups');
  tracing_ifs_code:write(umber_trace_file,'tracingifs');
  tracing_scan_tokens_code:write(umber_trace_file,'tracingscantokens');
  tracing_nesting_code:write(umber_trace_file,'tracingnesting');
  pre_display_direction_code:write(umber_trace_file,'predisplaydirection');
  last_line_fit_code:write(umber_trace_file,'lastlinefit');
  saving_vdiscards_code:write(umber_trace_file,'savingvdiscards');
  saving_hyph_codes_code:write(umber_trace_file,'savinghyphcodes');
  eTeX_state_code+TeXXeT_code:write(umber_trace_file,'TeXXeTstate');
  othercases write(umber_trace_file,'integer_parameter:',slot:1)
  endcases
else
case family of
0:write(umber_trace_file,'glue_parameter');
1:write(umber_trace_file,'skip');
2:write(umber_trace_file,'muskip');
3:write(umber_trace_file,'token_parameter');
4:write(umber_trace_file,'toks');
5:write(umber_trace_file,'lccode');
6:write(umber_trace_file,'uccode');
7:write(umber_trace_file,'sfcode');
8:write(umber_trace_file,'mathcode');
9:write(umber_trace_file,'integer_parameter');
10:write(umber_trace_file,'count');
11:write(umber_trace_file,'delcode');
12:write(umber_trace_file,'dimension_parameter');
othercases write(umber_trace_file,'dimen')
endcases;
if family<>9 then write(umber_trace_file,':',slot:1);
write(umber_trace_file,'"');
end;
@y
procedure umber_trace_named_slot(@!family:integer;@!slot:integer);
begin
write(umber_trace_file,'"');
if family=3 then
  case slot of
  pdf_pages_attr_loc-output_routine_loc:
    write(umber_trace_file,'pdfpagesattr');
  pdf_page_attr_loc-output_routine_loc:
    write(umber_trace_file,'pdfpageattr');
  pdf_page_resources_loc-output_routine_loc:
    write(umber_trace_file,'pdfpageresources');
  pdf_pk_mode_loc-output_routine_loc:
    write(umber_trace_file,'pdfpkmode');
  othercases write(umber_trace_file,'token_parameter:',slot:1)
  endcases
else if family=9 then
  case slot of
  pdf_output_code:write(umber_trace_file,'pdfoutput');
  pdf_compress_level_code:write(umber_trace_file,'pdfcompresslevel');
  pdf_decimal_digits_code:write(umber_trace_file,'pdfdecimaldigits');
  pdf_move_chars_code:write(umber_trace_file,'pdfmovechars');
  pdf_image_resolution_code:write(umber_trace_file,'pdfimageresolution');
  pdf_pk_resolution_code:write(umber_trace_file,'pdfpkresolution');
  pdf_unique_resname_code:write(umber_trace_file,'pdfuniqueresname');
  pdf_option_always_use_pdfpagebox_code:
    write(umber_trace_file,'pdfoptionalwaysusepdfpagebox');
  pdf_option_pdf_inclusion_errorlevel_code:
    write(umber_trace_file,'pdfoptionpdfinclusionerrorlevel');
  pdf_major_version_code:write(umber_trace_file,'pdfmajorversion');
  pdf_minor_version_code:write(umber_trace_file,'pdfminorversion');
  pdf_force_pagebox_code:write(umber_trace_file,'pdfforcepagebox');
  pdf_pagebox_code:write(umber_trace_file,'pdfpagebox');
  pdf_inclusion_errorlevel_code:
    write(umber_trace_file,'pdfinclusionerrorlevel');
  pdf_gamma_code:write(umber_trace_file,'pdfgamma');
  pdf_image_gamma_code:write(umber_trace_file,'pdfimagegamma');
  pdf_image_hicolor_code:write(umber_trace_file,'pdfimagehicolor');
  pdf_image_apply_gamma_code:write(umber_trace_file,'pdfimageapplygamma');
  pdf_adjust_spacing_code:write(umber_trace_file,'pdfadjustspacing');
  pdf_protrude_chars_code:write(umber_trace_file,'pdfprotrudechars');
  pdf_tracing_fonts_code:write(umber_trace_file,'pdftracingfonts');
  pdf_objcompresslevel_code:write(umber_trace_file,'pdfobjcompresslevel');
  pdf_adjust_interword_glue_code:
    write(umber_trace_file,'pdfadjustinterwordglue');
  pdf_prepend_kern_code:write(umber_trace_file,'pdfprependkern');
  pdf_append_kern_code:write(umber_trace_file,'pdfappendkern');
  pdf_gen_tounicode_code:write(umber_trace_file,'pdfgentounicode');
  pdf_draftmode_code:write(umber_trace_file,'pdfdraftmode');
  pdf_inclusion_copy_font_code:
    write(umber_trace_file,'pdfinclusioncopyfonts');
  pdf_suppress_warning_dup_dest_code:
    write(umber_trace_file,'pdfsuppresswarningdupdest');
  pdf_suppress_warning_dup_map_code:
    write(umber_trace_file,'pdfsuppresswarningdupmap');
  pdf_suppress_warning_page_group_code:
    write(umber_trace_file,'pdfsuppresswarningpagegroup');
  pdf_info_omit_date_code:write(umber_trace_file,'pdfinfoomitdate');
  pdf_suppress_ptex_info_code:
    write(umber_trace_file,'pdfsuppressptexinfo');
  pdf_omit_charset_code:write(umber_trace_file,'pdfomitcharset');
  pdf_omit_info_dict_code:write(umber_trace_file,'pdfomitinfodict');
  pdf_omit_procset_code:write(umber_trace_file,'pdfomitprocset');
  pdf_ptex_use_underscore_code:
    write(umber_trace_file,'pdfptexuseunderscore');
  tracing_assigns_code:write(umber_trace_file,'tracingassigns');
  tracing_groups_code:write(umber_trace_file,'tracinggroups');
  tracing_ifs_code:write(umber_trace_file,'tracingifs');
  tracing_scan_tokens_code:write(umber_trace_file,'tracingscantokens');
  tracing_nesting_code:write(umber_trace_file,'tracingnesting');
  pre_display_direction_code:write(umber_trace_file,'predisplaydirection');
  last_line_fit_code:write(umber_trace_file,'lastlinefit');
  saving_vdiscards_code:write(umber_trace_file,'savingvdiscards');
  saving_hyph_codes_code:write(umber_trace_file,'savinghyphcodes');
  ignore_primitive_error_code:write(umber_trace_file,'ignoreprimitiveerror');
  eTeX_state_code+TeXXeT_code:write(umber_trace_file,'TeXXeTstate');
  othercases write(umber_trace_file,'integer_parameter:',slot:1)
  endcases
else if family=12 then
  case slot of
  pdf_h_origin_code:write(umber_trace_file,'pdfhorigin');
  pdf_v_origin_code:write(umber_trace_file,'pdfvorigin');
  pdf_page_width_code:write(umber_trace_file,'pdfpagewidth');
  pdf_page_height_code:write(umber_trace_file,'pdfpageheight');
  pdf_link_margin_code:write(umber_trace_file,'pdflinkmargin');
  pdf_dest_margin_code:write(umber_trace_file,'pdfdestmargin');
  pdf_thread_margin_code:write(umber_trace_file,'pdfthreadmargin');
  pdf_first_line_height_code:write(umber_trace_file,'pdffirstlineheight');
  pdf_last_line_depth_code:write(umber_trace_file,'pdflastlinedepth');
  pdf_each_line_height_code:write(umber_trace_file,'pdfeachlineheight');
  pdf_each_line_depth_code:write(umber_trace_file,'pdfeachlinedepth');
  pdf_ignored_dimen_code:write(umber_trace_file,'pdfignoreddimen');
  pdf_px_dimen_code:write(umber_trace_file,'pdfpxdimen');
  othercases write(umber_trace_file,'dimension_parameter:',slot:1)
  endcases
else
  begin
  case family of
  0:write(umber_trace_file,'glue_parameter');
  1:write(umber_trace_file,'skip');
  2:write(umber_trace_file,'muskip');
  4:write(umber_trace_file,'toks');
  5:write(umber_trace_file,'lccode');
  6:write(umber_trace_file,'uccode');
  7:write(umber_trace_file,'sfcode');
  8:write(umber_trace_file,'mathcode');
  10:write(umber_trace_file,'count');
  11:write(umber_trace_file,'delcode');
  othercases write(umber_trace_file,'dimen')
  endcases;
  write(umber_trace_file,':',slot:1);
  end;
write(umber_trace_file,'"');
end;

procedure umber_trace_pdf_font_code(@!code_kind:integer;
  @!f:internal_font_number;@!char_code,@!value:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"mutation","data":{"target":"code_table",',
  '"key":{"type":"name","value":"');
case code_kind of
lp_code_base:write(umber_trace_file,'lpcode');
rp_code_base:write(umber_trace_file,'rpcode');
ef_code_base:write(umber_trace_file,'efcode');
tag_code:write(umber_trace_file,'tagcode');
kn_bs_code_base:write(umber_trace_file,'knbscode');
st_bs_code_base:write(umber_trace_file,'stbscode');
sh_bs_code_base:write(umber_trace_file,'shbscode');
kn_bc_code_base:write(umber_trace_file,'knbccode');
kn_ac_code_base:write(umber_trace_file,'knaccode');
othercases write(umber_trace_file,'pdfnoligatures')
endcases;
write(umber_trace_file,':');
umber_trace_string_contents(font_name[f]);
if char_code>=0 then write(umber_trace_file,':',char_code:1);
write(umber_trace_file,'"},"value":');
if code_kind=no_lig_code then
  write(umber_trace_file,'{"type":"bool","value":true}')
else write(umber_trace_file,'{"type":"integer","value":',value:1,'}');
write_ln(umber_trace_file,',"scope":"global"}}}');
end;

procedure umber_trace_pdf_enquiry(@!enquiry_kind,@!value:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"scanner","data":{"scanner":"');
case enquiry_kind of
0:write(umber_trace_file,'pdf_last_x_position');
1:write(umber_trace_file,'pdf_last_y_position');
2:write(umber_trace_file,'pdf_extension_result');
3:write(umber_trace_file,'pdf_elapsed_time');
4:write(umber_trace_file,'pdf_shell_escape');
5:write(umber_trace_file,'pdf_random_seed');
6:write(umber_trace_file,'pdf_font_name');
7:write(umber_trace_file,'pdf_font_size');
8:write(umber_trace_file,'pdf_left_margin_kern');
9:write(umber_trace_file,'pdf_right_margin_kern');
othercases write(umber_trace_file,'pdf_insertion_height')
endcases;
write(umber_trace_file,'","result":');
if enquiry_kind=3 then
  write(umber_trace_file,'{"type":"name","value":"host_timer_sample"}')
else if enquiry_kind=6 then
  begin
  write(umber_trace_file,'{"type":"name","value":"');
  umber_trace_string_contents(font_name[value]);
  write(umber_trace_file,'"}');
  end
else if (enquiry_kind=7)or(enquiry_kind>=8) then
  write(umber_trace_file,'{"type":"scaled","value":',value:1,'}')
else write(umber_trace_file,'{"type":"integer","value":',value:1,'}');
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_pdf_state(@!state_kind,@!value:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"mutation","data":{"target":"parameter",',
  '"key":{"type":"name","value":"');
if state_kind=0 then write(umber_trace_file,'pdfresettimer')
else if state_kind=1 then write(umber_trace_file,'pdfsetrandomseed')
else if state_kind=2 then write(umber_trace_file,'pdfinterwordspaceon')
else if state_kind=3 then write(umber_trace_file,'pdfinterwordspaceoff')
else if state_kind=4 then write(umber_trace_file,'pdffakespace')
else if state_kind=5 then write(umber_trace_file,'pdfrunninglinkoff')
else write(umber_trace_file,'pdfrunninglinkon');
write(umber_trace_file,'"},"value":');
if state_kind=0 then
  write(umber_trace_file,'{"type":"name","value":"reset"}')
else if state_kind=2 then
  write(umber_trace_file,'{"type":"name","value":"ordered-whatsit-on"}')
else if state_kind=3 then
  write(umber_trace_file,'{"type":"name","value":"ordered-whatsit-off"}')
else if state_kind=4 then
  write(umber_trace_file,'{"type":"name","value":"ordered-whatsit-fake"}')
else if state_kind=5 then
  write(umber_trace_file,'{"type":"name","value":"ordered-whatsit-off"}')
else if state_kind=6 then
  write(umber_trace_file,'{"type":"name","value":"ordered-whatsit-on"}')
else write(umber_trace_file,'{"type":"integer","value":',value:1,'}');
write_ln(umber_trace_file,',"scope":"global"}}}');
end;

procedure umber_trace_pdf_space_font(@!value:str_number);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"mutation","data":{"target":"parameter",',
  '"key":{"type":"name","value":"pdfspacefont"},',
  '"value":{"type":"name","value":"');
umber_trace_string_contents(value);
write_ln(umber_trace_file,'"},"scope":"global"}}}');
end;
@z

@x [18] Distinguish externally committed PDF and DVI page artifacts.
else if effect_kind=4 then write(umber_trace_file,'dvi')
else write(umber_trace_file,'stream:',channel:1);
@y
else if effect_kind=4 then
  if pdf_output>0 then write(umber_trace_file,'pdf')
  else write(umber_trace_file,'dvi')
else write(umber_trace_file,'stream:',channel:1);
@z

@x [18] Initialize the volatile-enquiry guard.
umber_alignment_depth:=0; umber_mutation_command:=false;
umber_line_shift:=0; umber_shift_tail:=0; umber_shift_pos:=0;
  rewrite(umber_trace_file,'pdftex14029-events.jsonl');
@y
umber_alignment_depth:=0; umber_mutation_command:=false;
umber_line_shift:=0; umber_shift_tail:=0; umber_shift_pos:=0;
umber_pdf_timer_enquiry:=false;
umber_pdf_object_tokens:=false;
  rewrite(umber_trace_file,'pdftex14029-events.jsonl');
@z

@x [25] Observe the operand consumed by \pdfprimitive.
begin save_scanner_status := scanner_status; scanner_status:=normal;
get_token; scanner_status:=save_scanner_status;
if cur_cs < hash_base then
@y
begin save_scanner_status := scanner_status; scanner_status:=normal;
get_token; scanner_status:=save_scanner_status;
umber_trace_token_splice(13,cur_tok);
if cur_cs < hash_base then
@z

@x [27] Observe non-early conversion results before input insertion.
selector:=old_setting; link(garbage):=str_toks(b); ins_list(link(temp_head));
exit:end;
@y
selector:=old_setting; link(garbage):=str_toks(b);
case c of
pdftex_revision_code:umber_trace_token_list(1,26,link(temp_head),null);
pdftex_banner_code:umber_trace_token_list(1,27,link(temp_head),null);
pdf_strcmp_code:umber_trace_token_list(1,30,link(temp_head),null);
uniform_deviate_code:umber_trace_token_list(1,28,link(temp_head),null);
normal_deviate_code:umber_trace_token_list(1,29,link(temp_head),null);
end;
ins_list(link(temp_head));
exit:end;
@z

@x [27] Observe the completed token list returned by \expanded.
    scan_pdf_ext_toks;
    warning_index := save_warning_index;
    scanner_status := save_scanner_status;
    ins_list(link(def_ref));
    free_avail(def_ref);
@y
    scan_pdf_ext_toks;
    warning_index := save_warning_index;
    scanner_status := save_scanner_status;
    umber_trace_token_list(1,14,link(def_ref),null);
    ins_list(link(def_ref));
    free_avail(def_ref);
@z

@x [27] Observe exact eight-bit escaped-string output.
    b := pool_ptr;
    escapestring(str_start[s]);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    escapestring(str_start[s]);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,15,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe exact eight-bit escaped-name output.
    b := pool_ptr;
    escapename(str_start[s]);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    escapename(str_start[s]);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,16,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe exact eight-bit hexadecimal output.
    b := pool_ptr;
    escapehex(str_start[s]);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    escapehex(str_start[s]);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,17,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe exact eight-bit hexadecimal decoding.
    b := pool_ptr;
    unescapehex(str_start[s]);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    unescapehex(str_start[s]);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,18,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe the fixed-clock creation-date conversion.
pdf_creation_date_code:
  begin
    b := pool_ptr;
    getcreationdate;
    link(garbage) := str_toks(b);
    ins_list(link(temp_head));
@y
pdf_creation_date_code:
  begin
    b := pool_ptr;
    getcreationdate;
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,19,link(temp_head),null);
    ins_list(link(temp_head));
@z

@x [27] Observe the file-modification-date conversion result.
    b := pool_ptr;
    getfilemoddate(s);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    getfilemoddate(s);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,20,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe the file-size conversion result.
    b := pool_ptr;
    getfilesize(s);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    getfilesize(s);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,21,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe text and file MD5 results without helper identity.
    b := pool_ptr;
    getmd5sum(s, booltemp);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    getmd5sum(s, booltemp);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,22,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe the bounded file-dump conversion result.
    b := pool_ptr;
    getfiledump(s, i, j);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    getfiledump(s, i, j);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,23,link(temp_head),null);
    flush_str(s);
@z

@x [27] Observe the regular-expression result.
    b := pool_ptr;
    matchstrings(s, t, i, booltemp);
    link(garbage) := str_toks(b);
    flush_str(t);
@y
    b := pool_ptr;
    matchstrings(s, t, i, booltemp);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,24,link(temp_head),null);
    flush_str(t);
@z

@x [27] Observe a committed regular-expression capture.
    b := pool_ptr;
    getmatch(cur_val);
    link(garbage) := str_toks(b);
    ins_list(link(temp_head));
@y
    b := pool_ptr;
    getmatch(cur_val);
    link(garbage) := str_toks(b);
    umber_trace_token_list(1,25,link(temp_head),null);
    ins_list(link(temp_head));
@z

@x [53a] Observe the completed bytewise string comparison.
    flush_str(s2);
    flush_str(s1);
    cur_val_level := int_val;
end;
@y
    flush_str(s2);
    flush_str(s1);
    cur_val_level := int_val;
    umber_trace_scanner(20,int_val);
end;
@z
