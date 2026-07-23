% pdfTeX 1.40.27 command-extension observation layered after instrumentation.ch.
% The transport and schema helpers are owned by the preceding change file.

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
    getmd5sum(s, bool);
    link(garbage) := str_toks(b);
    flush_str(s);
@y
    b := pool_ptr;
    getmd5sum(s, bool);
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
    matchstrings(s, t, i, bool);
    link(garbage) := str_toks(b);
    flush_str(t);
@y
    b := pool_ptr;
    matchstrings(s, t, i, bool);
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
