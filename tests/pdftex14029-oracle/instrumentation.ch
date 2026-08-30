% Final transparent base command-core instrumentation for canonical pdfTeX 1.40.29.
% The observer owns only a detached text file, a sequence counter, and flags.

@x [4] Observer-owned globals.
@!str_ptr : str_number; {number of the current string being created}
@!init_pool_ptr : pool_pointer; {the starting value of |pool_ptr|}
@y
@!str_ptr : str_number; {number of the current string being created}
@!init_pool_ptr : pool_pointer; {the starting value of |pool_ptr|}
@!umber_trace_file:alpha_file;
@!umber_trace_opened:boolean;
@!umber_trace_sequence:integer;
@!umber_diag_file:alpha_file;
@!umber_diag_opened:boolean;
@!umber_diag_sequence:integer;
@!umber_diag_has_report:boolean;
@!umber_diag_has_location:boolean;
@!umber_diag_source:str_number;
@!umber_diag_line:integer;
@!umber_diag_byte:integer;
@!umber_recovery_insert:boolean;
@!umber_alignment_depth:integer;
@!umber_mutation_command:boolean;
@!umber_line_shift:integer;
@!umber_shift_tail:integer;
@!umber_shift_pos:integer;
@!umber_shift_stack:array[1..sup_max_in_open] of integer;
@!umber_tail_stack:array[1..sup_max_in_open] of integer;
@!umber_pos_stack:array[1..sup_max_in_open] of integer;
@z

@x [18] Observe central fatal-error procedure.
begin normalize_selector;@/
print_err("Emergency stop"); help1(s); succumb;
@y
begin normalize_selector;@/
print_err("Emergency stop"); umber_diag_report(2,"emergency-stop",s);
help1(s); succumb;
@z

@x [18] Observe central overflow procedure.
print(s); print_char("="); print_int(n); print_char("]");
help2("If you really absolutely need more capacity,")@/
@y
print(s); print_char("="); print_int(n); print_char("]");
umber_diag_report(2,"capacity-exceeded",s);
help2("If you really absolutely need more capacity,")@/
@z

@x [18] Observe central confusion procedure.
begin normalize_selector;
if history<error_message_issued then
@y
begin normalize_selector;
umber_diag_report(2,"confusion",s);
if history<error_message_issued then
@z

@x [22] Detached schema-v1 JSON Lines transport.
procedure sprint_cs(@!p:pointer); {prints a control sequence}
begin if p<hash_base then
  if p<single_base then print(p-active_base)
  else  if p<null_cs then print_esc(p-single_base)
    else  begin print_esc("csname"); print_esc("endcsname");
      end
else if (p>=prim_eqtb_base)and(p<frozen_null_font) then
    print_esc(prim_text(p-prim_eqtb_base)-1)
else print_esc(text(p));
end;
@y
procedure sprint_cs(@!p:pointer); {prints a control sequence}
begin if p<hash_base then
  if p<single_base then print(p-active_base)
  else  if p<null_cs then print_esc(p-single_base)
    else  begin print_esc("csname"); print_esc("endcsname");
      end
else if (p>=prim_eqtb_base)and(p<frozen_null_font) then
    print_esc(prim_text(p-prim_eqtb_base)-1)
else print_esc(text(p));
end;

procedure umber_trace_hex(@!c:integer);
var d:integer;
begin
{|xchr|, not |chr|: web2c compiles Pascal's identity |chr| away, so
|write(f,chr(n))| emits the decimal number |n| instead of that character.}
d:=c div 16;
if d<10 then write(umber_trace_file,xchr[d+48])
else write(umber_trace_file,xchr[d+87]);
d:=c mod 16;
if d<10 then write(umber_trace_file,xchr[d+48])
else write(umber_trace_file,xchr[d+87]);
end;

{JSON has exactly one canonical spelling per character, and the fixture
contract requires the stream to round-trip through it byte for byte: the two
mandatory escapes, the five short control escapes, |\u00xx| for the remaining
C0 controls, and the character itself otherwise. |DEL| needs no escape, and
codes 128-255 denote U+0080-U+00FF, whose canonical form is their two-byte
UTF-8 encoding rather than a |\u00xx| escape.}
procedure umber_trace_char(@!c:integer);
begin
if c=34 then write(umber_trace_file,'\"')
else if c=92 then write(umber_trace_file,'\\')
else if c=8 then write(umber_trace_file,'\b')
else if c=9 then write(umber_trace_file,'\t')
else if c=10 then write(umber_trace_file,'\n')
else if c=12 then write(umber_trace_file,'\f')
else if c=13 then write(umber_trace_file,'\r')
else if c<32 then
  begin write(umber_trace_file,'\u00'); umber_trace_hex(c mod 256); end
else if c<128 then write(umber_trace_file,xchr[c])
else begin
  write(umber_trace_file,xchr[192+((c mod 256) div 64)]);
  write(umber_trace_file,xchr[128+(c mod 64)]);
  end;
end;

procedure umber_trace_string_contents(@!s:str_number);
var k:pool_pointer;
begin for k:=str_start[s] to str_start[s+1]-1 do
  umber_trace_char(so(str_pool[k]));
end;

procedure umber_trace_string(@!s:str_number);
begin write(umber_trace_file,'"'); umber_trace_string_contents(s);
write(umber_trace_file,'"');
end;

procedure umber_trace_cs(@!p:pointer);
begin
write(umber_trace_file,'"');
if p<hash_base then
  if p<single_base then umber_trace_char(p-active_base)
  else if p<null_cs then umber_trace_char(p-single_base)
  else write(umber_trace_file,'csname\endcsname')
else umber_trace_string_contents(text(p));
write(umber_trace_file,'"');
end;

procedure umber_trace_begin;
begin write(umber_trace_file,'{"sequence":',umber_trace_sequence:1,
  ',"semantic":'); incr(umber_trace_sequence);
end;

procedure umber_trace_command_name(@!c:integer);
begin
case c of
0:write(umber_trace_file,'"relax"');
1:write(umber_trace_file,'"left_brace"');
2:write(umber_trace_file,'"right_brace"');
3:write(umber_trace_file,'"math_shift"');
4:write(umber_trace_file,'"tab_mark"');
5:write(umber_trace_file,'"car_ret"');
6:write(umber_trace_file,'"mac_param"');
7:write(umber_trace_file,'"sup_mark"');
8:write(umber_trace_file,'"sub_mark"');
9:write(umber_trace_file,'"endv"');
10:write(umber_trace_file,'"spacer"');
11:write(umber_trace_file,'"letter"');
12:write(umber_trace_file,'"other_char"');
13:write(umber_trace_file,'"par_end"');
14:write(umber_trace_file,'"stop"');
15:write(umber_trace_file,'"delim_num"');
16:write(umber_trace_file,'"char_num"');
17:write(umber_trace_file,'"math_char_num"');
18:write(umber_trace_file,'"mark"');
19:write(umber_trace_file,'"xray"');
20:write(umber_trace_file,'"make_box"');
21:write(umber_trace_file,'"hmove"');
22:write(umber_trace_file,'"vmove"');
23:write(umber_trace_file,'"un_hbox"');
24:write(umber_trace_file,'"un_vbox"');
25:write(umber_trace_file,'"remove_item"');
26:write(umber_trace_file,'"hskip"');
27:write(umber_trace_file,'"vskip"');
28:write(umber_trace_file,'"mskip"');
29:write(umber_trace_file,'"kern"');
30:write(umber_trace_file,'"mkern"');
31:write(umber_trace_file,'"leader_ship"');
32:write(umber_trace_file,'"halign"');
33:write(umber_trace_file,'"valign"');
34:write(umber_trace_file,'"no_align"');
35:write(umber_trace_file,'"vrule"');
36:write(umber_trace_file,'"hrule"');
37:write(umber_trace_file,'"insert"');
38:write(umber_trace_file,'"vadjust"');
39:write(umber_trace_file,'"ignore_spaces"');
40:write(umber_trace_file,'"after_assignment"');
41:write(umber_trace_file,'"after_group"');
42:write(umber_trace_file,'"break_penalty"');
43:write(umber_trace_file,'"start_par"');
44:write(umber_trace_file,'"ital_corr"');
45:write(umber_trace_file,'"accent"');
46:write(umber_trace_file,'"math_accent"');
47:write(umber_trace_file,'"discretionary"');
48:write(umber_trace_file,'"eq_no"');
49:write(umber_trace_file,'"left_right"');
50:write(umber_trace_file,'"math_comp"');
51:write(umber_trace_file,'"limit_switch"');
52:write(umber_trace_file,'"above"');
53:write(umber_trace_file,'"math_style"');
54:write(umber_trace_file,'"math_choice"');
55:write(umber_trace_file,'"non_script"');
56:write(umber_trace_file,'"vcenter"');
57:write(umber_trace_file,'"case_shift"');
58:write(umber_trace_file,'"message"');
59:write(umber_trace_file,'"extension"');
60:write(umber_trace_file,'"in_stream"');
61:write(umber_trace_file,'"begin_group"');
62:write(umber_trace_file,'"end_group"');
63:write(umber_trace_file,'"omit"');
64:write(umber_trace_file,'"ex_space"');
65:write(umber_trace_file,'"no_boundary"');
66:write(umber_trace_file,'"radical"');
67:write(umber_trace_file,'"end_cs_name"');
68:write(umber_trace_file,'"char_given"');
69:write(umber_trace_file,'"math_given"');
70:write(umber_trace_file,'"last_item"');
71:write(umber_trace_file,'"toks_register"');
72:write(umber_trace_file,'"assign_toks"');
73:write(umber_trace_file,'"assign_int"');
74:write(umber_trace_file,'"assign_dimen"');
75:write(umber_trace_file,'"assign_glue"');
76:write(umber_trace_file,'"assign_mu_glue"');
77:write(umber_trace_file,'"assign_font_dimen"');
78:write(umber_trace_file,'"assign_font_int"');
79:write(umber_trace_file,'"set_aux"');
80:write(umber_trace_file,'"set_prev_graf"');
81:write(umber_trace_file,'"set_page_dimen"');
82:write(umber_trace_file,'"set_page_int"');
83:write(umber_trace_file,'"set_box_dimen"');
84:write(umber_trace_file,'"set_shape"');
85:write(umber_trace_file,'"def_code"');
86:write(umber_trace_file,'"def_family"');
87:write(umber_trace_file,'"set_font"');
88:write(umber_trace_file,'"def_font"');
89:write(umber_trace_file,'"register"');
90:write(umber_trace_file,'"advance"');
91:write(umber_trace_file,'"multiply"');
92:write(umber_trace_file,'"divide"');
93:write(umber_trace_file,'"prefix"');
94:write(umber_trace_file,'"let"');
95:write(umber_trace_file,'"shorthand_def"');
96:write(umber_trace_file,'"read_to_cs"');
97:write(umber_trace_file,'"def"');
98:write(umber_trace_file,'"set_box"');
99:write(umber_trace_file,'"hyph_data"');
100:write(umber_trace_file,'"set_interaction"');
101:write(umber_trace_file,'"undefined_cs"');
102:write(umber_trace_file,'"expand_after"');
103:write(umber_trace_file,'"no_expand"');
104:write(umber_trace_file,'"input"');
105:write(umber_trace_file,'"if_test"');
106:write(umber_trace_file,'"fi_or_else"');
107:write(umber_trace_file,'"cs_name"');
108:write(umber_trace_file,'"convert"');
109:write(umber_trace_file,'"the"');
110:write(umber_trace_file,'"top_bot_mark"');
111:write(umber_trace_file,'"call"');
112:write(umber_trace_file,'"long_call"');
113:write(umber_trace_file,'"outer_call"');
114:write(umber_trace_file,'"long_outer_call"');
115:write(umber_trace_file,'"end_template"');
116:write(umber_trace_file,'"dont_expand"');
othercases write(umber_trace_file,'"unknown_command"')
endcases;
end;

procedure umber_trace_catcode(@!c:integer);
begin case c of
0:write(umber_trace_file,'"escape"');
1:write(umber_trace_file,'"left_brace"');
2:write(umber_trace_file,'"right_brace"');
3:write(umber_trace_file,'"math_shift"');
4:write(umber_trace_file,'"tab_mark"');
5:write(umber_trace_file,'"car_ret"');
6:write(umber_trace_file,'"mac_param"');
7:write(umber_trace_file,'"sup_mark"');
8:write(umber_trace_file,'"sub_mark"');
9:write(umber_trace_file,'"ignore"');
10:write(umber_trace_file,'"spacer"');
11:write(umber_trace_file,'"letter"');
12:write(umber_trace_file,'"other_char"');
13:write(umber_trace_file,'"active_char"');
14:write(umber_trace_file,'"comment"');
15:write(umber_trace_file,'"invalid_char"');
othercases write(umber_trace_file,'"escape"')
endcases;
end;

function umber_source_column(@!p:integer):integer;
begin
if p>=umber_shift_pos then umber_source_column:=p+umber_line_shift
else umber_source_column:=p+umber_line_shift-umber_shift_tail;
end;

procedure umber_diag_hex(@!c:integer);
var d:integer;
begin d:=c div 16;
if d<10 then write(umber_diag_file,xchr[d+48])
else write(umber_diag_file,xchr[d+87]);
d:=c mod 16;
if d<10 then write(umber_diag_file,xchr[d+48])
else write(umber_diag_file,xchr[d+87]);
end;

procedure umber_diag_char(@!c:integer);
begin
if c=34 then write(umber_diag_file,'\"')
else if c=92 then write(umber_diag_file,'\\')
else if c=8 then write(umber_diag_file,'\b')
else if c=9 then write(umber_diag_file,'\t')
else if c=10 then write(umber_diag_file,'\n')
else if c=12 then write(umber_diag_file,'\f')
else if c=13 then write(umber_diag_file,'\r')
else if c<32 then
  begin write(umber_diag_file,'\u00'); umber_diag_hex(c mod 256); end
else if c<128 then write(umber_diag_file,xchr[c])
else begin
  write(umber_diag_file,xchr[192+((c mod 256) div 64)]);
  write(umber_diag_file,xchr[128+(c mod 64)]);
  end;
end;

procedure umber_diag_string(@!s:str_number);
var k:pool_pointer;
begin write(umber_diag_file,'"');
for k:=str_start[s] to str_start[s+1]-1 do umber_diag_char(so(str_pool[k]));
write(umber_diag_file,'"');
end;

procedure umber_diag_begin;
begin write(umber_diag_file,'{"sequence":',umber_diag_sequence:1,
  ',"semantic":'); incr(umber_diag_sequence);
end;

procedure umber_diag_remember_location;
begin
if (state<>0)and(name>17) then
  begin umber_diag_has_location:=true; umber_diag_source:=name;
  umber_diag_line:=line;
  if loc>start then umber_diag_byte:=umber_source_column(loc-start-1)
  else umber_diag_byte:=0;
  end;
end;

{Kinds: 0 recoverable error, 1 warning, 2 fatal. An empty argument string
means the typed argument vector is empty; rendered help and context never
enter this detached stream.}
procedure umber_diag_report(@!kind:integer;@!diagnostic,@!argument:str_number);
begin
if (not umber_diag_opened)or(not umber_diag_has_location) then return;
umber_diag_has_report:=true;
umber_diag_begin;
write(umber_diag_file,'{"event":"diagnostic_lifecycle",');
write(umber_diag_file,'"data":{"transition":"report","class":"');
if kind=0 then write(umber_diag_file,'recoverable_error')
else if kind=1 then write(umber_diag_file,'warning')
else write(umber_diag_file,'fatal');
write(umber_diag_file,'","severity":"');
if kind=0 then write(umber_diag_file,'error')
else if kind=1 then write(umber_diag_file,'warning')
else write(umber_diag_file,'fatal');
write(umber_diag_file,'","diagnostic":'); umber_diag_string(diagnostic);
write(umber_diag_file,',"arguments":[');
if argument<>0 then
  begin write(umber_diag_file,'{"type":"name","value":');
  umber_diag_string(argument); write(umber_diag_file,'}'); end;
write(umber_diag_file,'],"location":{"source":');
umber_diag_string(umber_diag_source);
write_ln(umber_diag_file,',"line":',umber_diag_line:1,
  ',"byte":',umber_diag_byte:1,'}}}}');
if kind=2 then
  begin
  umber_diag_begin;
  write_ln(umber_diag_file,
    '{"event":"diagnostic_lifecycle","data":{"transition":"outcome",',
    '"history":"fatal_error_stop","outcome":"aborted"}}}');
  a_close(umber_diag_file); umber_diag_opened:=false;
  end;
end;

{The emitted |byte| is a column of the immutable source line, not an index
into |buffer|. \S355 reduces an expanded code inside a control-sequence name
in place and shifts later bytes down by |d|. The accumulated shift is added
back for a byte at or after the reduced code, but not for a command ending
before a collapse performed only by \S356 lookahead.}
procedure umber_trace_command(@!expanded:boolean);
begin
if not umber_trace_opened then return;
umber_diag_remember_location;
umber_trace_begin;
write(umber_trace_file,'{"event":"command","data":{"delivery":"');
if expanded then write(umber_trace_file,'expanded')
else write(umber_trace_file,'raw');
write(umber_trace_file,'","command":{"command":');
umber_trace_command_name(cur_cmd);
write(umber_trace_file,',"operand":{"type":"integer","value":',
  cur_chr:1,'}');
if cur_cs<>0 then
  begin write(umber_trace_file,',"control_sequence":');
  umber_trace_cs(cur_cs); end;
if (state<>0)and(name>17) then
  begin write(umber_trace_file,',"location":{"source":');
  umber_trace_string(name);
  write(umber_trace_file,',"line":',line:1,',"byte":');
  if loc>start then
    write(umber_trace_file,umber_source_column(loc-start-1):1)
  else write(umber_trace_file,'0');
  write(umber_trace_file,'}'); end;
write_ln(umber_trace_file,'}}}}');
end;

procedure umber_trace_input_name(@!t:integer);
begin case t of
0:write(umber_trace_file,'"parameter"');
1:write(umber_trace_file,'"u_template"');
2:write(umber_trace_file,'"v_template"');
3:write(umber_trace_file,'"backup"');
4:write(umber_trace_file,'"recovery"');
5:write(umber_trace_file,'"macro"');
6:write(umber_trace_file,'"output"');
7:write(umber_trace_file,'"every_par"');
8:write(umber_trace_file,'"every_math"');
9:write(umber_trace_file,'"every_display"');
10:write(umber_trace_file,'"every_hbox"');
11:write(umber_trace_file,'"every_vbox"');
12:write(umber_trace_file,'"every_job"');
13:write(umber_trace_file,'"every_cr"');
14:write(umber_trace_file,'"mark"');
15:write(umber_trace_file,'"every_eof"');
16:write(umber_trace_file,'"write"');
othercases write(umber_trace_file,'"token_list"')
endcases;
end;

procedure umber_trace_input(@!transition,@!reason,@!t:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"input","data":{"transition":"');
if transition=0 then write(umber_trace_file,'push')
else if transition=1 then write(umber_trace_file,'retire')
else write(umber_trace_file,'stop');
write(umber_trace_file,'","reason":"');
case reason of
0:write(umber_trace_file,'source');
1:write(umber_trace_file,'token_list');
2:write(umber_trace_file,'macro');
3:write(umber_trace_file,'alignment_template');
4:write(umber_trace_file,'backup');
othercases write(umber_trace_file,'recovery')
endcases;
write(umber_trace_file,'","name":');
if reason=0 then
  if name>17 then umber_trace_string(name)
  else if name=0 then write(umber_trace_file,'"terminal"')
  else write(umber_trace_file,'"', 'r','e','a','d','_','s','t','r','e','a','m','"')
else umber_trace_input_name(t);
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_token(@!t:halfword);
var c:integer;
begin
if (t>@'2400)and(t<=@'2400+9) then
  write(umber_trace_file,'{"character":',t-@'2400:1,
    ',"catcode":"out_parameter"}')
else if (t>=@'6400)and(t<@'7000) then
  write(umber_trace_file,'{"character":',t-@'6400:1,
    ',"catcode":"match"}')
else if t=@'7000 then
  write(umber_trace_file,'{"character":0,"catcode":"end_match"}')
else if t>=cs_token_flag then
  begin write(umber_trace_file,
    '{"character":0,"catcode":"escape","control_sequence":');
  umber_trace_cs(t-cs_token_flag); write(umber_trace_file,'}'); end
else begin c:=t div @'400;
  write(umber_trace_file,'{"character":',t mod @'400:1,',"catcode":');
  umber_trace_catcode(c); write(umber_trace_file,'}'); end;
end;

procedure umber_trace_token_range(@!p,@!stop_pointer:pointer);
var first_token:boolean;
begin first_token:=true;
while p<>stop_pointer do
  begin
  if first_token then first_token:=false
  else write(umber_trace_file,',');
  umber_trace_token(info(p)); p:=link(p);
  end;
end;

procedure umber_trace_token_list(@!transition,@!purpose:integer;
  @!p,@!stop_pointer:pointer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"token_list","data":{"transition":"');
if transition=0 then write(umber_trace_file,'splice')
else write(umber_trace_file,'complete');
write(umber_trace_file,'","purpose":"');
case purpose of
0:write(umber_trace_file,'macro_delimiter_match');
1:write(umber_trace_file,'macro_delimiter_recovery');
2:write(umber_trace_file,'parameter_conversion');
3:write(umber_trace_file,'macro_replacement');
4:write(umber_trace_file,'scan_toks');
5:write(umber_trace_file,'expanded_scan_toks');
6:write(umber_trace_file,'the_toks');
7:write(umber_trace_file,'unexpanded');
8:write(umber_trace_file,'detokenize');
9:write(umber_trace_file,'scantokens');
10:write(umber_trace_file,'protected_macro');
11:write(umber_trace_file,'protected_expansion_suppression');
12:write(umber_trace_file,'protected_delivery_suppression');
13:write(umber_trace_file,'primitive_enquiry');
14:write(umber_trace_file,'pdf_expanded');
15:write(umber_trace_file,'pdf_escape_string');
16:write(umber_trace_file,'pdf_escape_name');
17:write(umber_trace_file,'pdf_escape_hex');
18:write(umber_trace_file,'pdf_unescape_hex');
19:write(umber_trace_file,'pdf_creation_date');
20:write(umber_trace_file,'pdf_file_mod_date');
21:write(umber_trace_file,'pdf_file_size');
22:write(umber_trace_file,'pdf_mdfive_sum');
23:write(umber_trace_file,'pdf_file_dump');
24:write(umber_trace_file,'pdf_match');
25:write(umber_trace_file,'pdf_last_match');
26:write(umber_trace_file,'pdftex_revision');
27:write(umber_trace_file,'pdftex_banner');
28:write(umber_trace_file,'pdf_uniform_deviate');
29:write(umber_trace_file,'pdf_normal_deviate');
othercases write(umber_trace_file,'pdf_conversion')
endcases;
write(umber_trace_file,'","tokens":[');
umber_trace_token_range(p,stop_pointer);
write_ln(umber_trace_file,']}}}');
end;

procedure umber_trace_token_splice(@!purpose:integer;@!t:halfword);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"token_list","data":{"transition":"splice","purpose":"');
case purpose of
1:write(umber_trace_file,'macro_delimiter_recovery');
2:write(umber_trace_file,'parameter_conversion');
11:write(umber_trace_file,'protected_expansion_suppression');
12:write(umber_trace_file,'protected_delivery_suppression');
13:write(umber_trace_file,'primitive_enquiry');
othercases write(umber_trace_file,'the_toks')
endcases;
write(umber_trace_file,'","tokens":['); umber_trace_token(t);
write_ln(umber_trace_file,']}}}');
end;

procedure umber_trace_macro_argument(@!parameter_number:integer;@!p:pointer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"macro","data":{"transition":"argument","parameter":',
  parameter_number:1,',"tokens":[');
umber_trace_token_range(p,null);
write_ln(umber_trace_file,']}}}');
end;

procedure umber_trace_macro_activation(@!cs:pointer;@!arguments:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"macro","data":{"transition":"activation",',
  '"control_sequence":');
umber_trace_cs(cs);
write_ln(umber_trace_file,',"argument_count":',arguments:1,'}}}');
end;

procedure umber_trace_glue_order(@!order:integer);
begin case order of
0:write(umber_trace_file,'"normal"');
1:write(umber_trace_file,'"fil"');
2:write(umber_trace_file,'"fill"');
othercases write(umber_trace_file,'"filll"')
endcases;
end;

procedure umber_trace_scanner(@!scanner_kind,@!value_level:integer);
var p:pointer;
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"scanner","data":{"scanner":"');
case scanner_kind of
0:write(umber_trace_file,'integer');
1:write(umber_trace_file,'dimension');
2:write(umber_trace_file,'glue');
3:write(umber_trace_file,'internal');
4:write(umber_trace_file,'expression_integer');
5:write(umber_trace_file,'expression_dimension');
6:write(umber_trace_file,'expression_glue');
7:write(umber_trace_file,'expression_muglue');
8:write(umber_trace_file,'current_group_level');
9:write(umber_trace_file,'current_group_type');
10:write(umber_trace_file,'current_condition_level');
11:write(umber_trace_file,'current_condition_type');
12:write(umber_trace_file,'current_condition_branch');
13:write(umber_trace_file,'glue_stretch_order');
14:write(umber_trace_file,'glue_shrink_order');
15:write(umber_trace_file,'glue_stretch');
16:write(umber_trace_file,'glue_shrink');
17:write(umber_trace_file,'mu_to_glue');
18:write(umber_trace_file,'glue_to_mu');
19:write(umber_trace_file,'interaction_mode');
othercases write(umber_trace_file,'pdf_string_compare')
endcases;
write(umber_trace_file,'","result":');
case value_level of
0:write(umber_trace_file,'{"type":"integer","value":',cur_val:1,'}');
1:write(umber_trace_file,'{"type":"scaled","value":',cur_val:1,'}');
2,3:begin p:=cur_val;
  write(umber_trace_file,'{"type":"glue","value":{"width":',width(p):1,
    ',"stretch":',stretch(p):1,',"stretch_order":');
  umber_trace_glue_order(stretch_order(p));
  write(umber_trace_file,',"shrink":',shrink(p):1,',"shrink_order":');
  umber_trace_glue_order(shrink_order(p));
  write(umber_trace_file,'}}');
  end;
4:begin write(umber_trace_file,'{"type":"name","value":');
  umber_trace_cs(cur_val); write(umber_trace_file,'}'); end;
othercases begin write(umber_trace_file,'{"type":"tokens","value":[');
  if cur_val<>null then umber_trace_token_range(link(cur_val),null);
  write(umber_trace_file,']}'); end
endcases;
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_condition_name(@!condition_kind:integer);
var inverted:boolean;
begin
inverted:=condition_kind>=32;
if inverted then
  begin write(umber_trace_file,'"unless_');
  condition_kind:=condition_kind mod 32;
  end
else write(umber_trace_file,'"');
case condition_kind of
0:write(umber_trace_file,'if');
1:write(umber_trace_file,'ifcat');
2:write(umber_trace_file,'ifnum');
3:write(umber_trace_file,'ifdim');
4:write(umber_trace_file,'ifodd');
5:write(umber_trace_file,'ifvmode');
6:write(umber_trace_file,'ifhmode');
7:write(umber_trace_file,'ifmmode');
8:write(umber_trace_file,'ifinner');
9:write(umber_trace_file,'ifvoid');
10:write(umber_trace_file,'ifhbox');
11:write(umber_trace_file,'ifvbox');
12:write(umber_trace_file,'ifx');
13:write(umber_trace_file,'ifeof');
14:write(umber_trace_file,'iftrue');
15:write(umber_trace_file,'iffalse');
16:write(umber_trace_file,'ifcase');
17:write(umber_trace_file,'ifdefined');
18:write(umber_trace_file,'ifcsname');
19:write(umber_trace_file,'iffontchar');
20:write(umber_trace_file,'ifincsname');
21:write(umber_trace_file,'ifpdfprimitive');
22:write(umber_trace_file,'ifpdfabsnum');
othercases write(umber_trace_file,'ifpdfabsdim')
endcases;
write(umber_trace_file,'"');
end;

procedure umber_trace_if_limit(@!limit_kind:integer);
begin case limit_kind of
1:write(umber_trace_file,'"evaluating"');
2:write(umber_trace_file,'"fi"');
3:write(umber_trace_file,'"else"');
4:write(umber_trace_file,'"or"');
othercases write(umber_trace_file,'"normal"')
endcases;
end;

procedure umber_trace_condition(@!transition,@!condition_kind,
  @!limit_kind,@!branch_kind:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"condition","data":{"transition":"');
case transition of
0:write(umber_trace_file,'push');
1:write(umber_trace_file,'limit_change');
2:write(umber_trace_file,'branch');
othercases write(umber_trace_file,'pop')
endcases;
write(umber_trace_file,'","condition":');
umber_trace_condition_name(condition_kind);
write(umber_trace_file,',"limit":');
umber_trace_if_limit(limit_kind);
if branch_kind<>0 then
  begin write(umber_trace_file,',"branch":"');
  case branch_kind of
  1:write(umber_trace_file,'true');
  2:write(umber_trace_file,'false');
  3:write(umber_trace_file,'or');
  4:write(umber_trace_file,'else');
  5:write(umber_trace_file,'fi');
  othercases write(umber_trace_file,'case')
  endcases;
  write(umber_trace_file,'"');
  end;
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_conditional_diagnostic(@!diagnostic_kind:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"diagnostic","data":{"severity":"error","diagnostic":"');
if diagnostic_kind=0 then write(umber_trace_file,'conditional_incomplete')
else if diagnostic_kind=1 then
  write(umber_trace_file,'conditional_limit_recovery')
else write(umber_trace_file,'conditional_extra_delimiter');
write_ln(umber_trace_file,'","arguments":[]}}}');
end;

procedure umber_trace_recovery(@!kind:integer;@!t:halfword);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"recovery","data":{"kind":"');
if kind=0 then write(umber_trace_file,'backup')
else if kind=1 then write(umber_trace_file,'inserted_token')
else write(umber_trace_file,'inserted_control_sequence');
write(umber_trace_file,'","tokens":['); umber_trace_token(t);
write_ln(umber_trace_file,']}}}');
end;

procedure umber_trace_off_save(@!at_bottom:boolean;@!t:halfword);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"diagnostic","data":{"severity":"error","diagnostic":"');
if at_bottom then write(umber_trace_file,'off_save_bottom_drop')
else write(umber_trace_file,'off_save_replay');
write(umber_trace_file,'","arguments":[{"type":"token","value":');
umber_trace_token(t);
write_ln(umber_trace_file,'}]}}}');
end;

procedure umber_trace_status(@!old_status,@!new_status:integer);
begin
if (not umber_trace_opened)or(old_status=new_status) then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"scanner_status","data":{"from":');
case old_status of
0:write(umber_trace_file,'"normal"'); 1:write(umber_trace_file,'"skipping"');
2:write(umber_trace_file,'"defining"'); 3:write(umber_trace_file,'"matching"');
4:write(umber_trace_file,'"aligning"');
othercases write(umber_trace_file,'"absorbing"') endcases;
write(umber_trace_file,',"to":');
case new_status of
0:write(umber_trace_file,'"normal"'); 1:write(umber_trace_file,'"skipping"');
2:write(umber_trace_file,'"defining"'); 3:write(umber_trace_file,'"matching"');
4:write(umber_trace_file,'"aligning"');
othercases write(umber_trace_file,'"absorbing"') endcases;
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_alignment(@!transition,@!old_state,
  @!template_kind:integer);
begin
if (not umber_trace_opened)or(umber_alignment_depth=0) then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"alignment","data":{"transition":"');
case transition of
0:write(umber_trace_file,'begin');
1:write(umber_trace_file,'finish');
2:write(umber_trace_file,'suspend');
3:write(umber_trace_file,'resume');
4:write(umber_trace_file,'preamble_start');
5:write(umber_trace_file,'preamble_finish');
6:write(umber_trace_file,'state_change');
7:write(umber_trace_file,'template_push');
8:write(umber_trace_file,'template_retire');
othercases write(umber_trace_file,'backup_correction')
endcases;
write(umber_trace_file,'","align_state":',align_state:1);
if template_kind<>0 then
  begin write(umber_trace_file,',"template":"');
  if template_kind=1 then write(umber_trace_file,'u')
  else if template_kind=2 then write(umber_trace_file,'v')
  else write(umber_trace_file,'omit');
  write(umber_trace_file,'"');
  end;
write(umber_trace_file,',"nesting":',umber_alignment_depth:1);
if old_state<>align_state then
  write(umber_trace_file,',"previous_align_state":',old_state:1);
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_alignment_delimiter(@!command_code,
  @!character_code:integer);
begin
if (not umber_trace_opened)or(umber_alignment_depth=0) then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"alignment","data":{"transition":"delimiter","align_state":',
  align_state:1,',"nesting":',umber_alignment_depth:1,',"delimiter":"');
if command_code=4 then
  if character_code=256 then write(umber_trace_file,'span')
  else write(umber_trace_file,'tab')
else if character_code=258 then write(umber_trace_file,'crcr')
else write(umber_trace_file,'cr');
write_ln(umber_trace_file,'"}}}');
end;

procedure umber_trace_alignment_recovery(@!recovery_kind:integer);
begin
if (not umber_trace_opened)or(umber_alignment_depth=0) then return;
umber_trace_begin;
write(umber_trace_file,
  '{"event":"alignment","data":{"transition":"recovery","align_state":',
  align_state:1,',"nesting":',umber_alignment_depth:1,',"recovery":"');
case recovery_kind of
0:write(umber_trace_file,'missing_parameter');
1:write(umber_trace_file,'extra_parameter');
2:write(umber_trace_file,'missing_left_brace');
3:write(umber_trace_file,'missing_right_brace');
4:write(umber_trace_file,'extra_tab');
othercases write(umber_trace_file,'outer_validity')
endcases;
write_ln(umber_trace_file,'"}}}');
end;

procedure umber_set_scanner_status(@!new_status:integer);
var old_status:integer;
begin old_status:=scanner_status; scanner_status:=new_status;
umber_trace_status(old_status,new_status);
end;

procedure umber_trace_outer(@!at_eof:boolean);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,
 '{"event":"diagnostic","data":{"severity":"error","diagnostic":"');
if at_eof then write(umber_trace_file,'outer_validity_eof')
else write(umber_trace_file,'outer_validity_control_sequence');
write_ln(umber_trace_file,'","arguments":[]}}}');
end;

procedure umber_trace_scope(@!global_scope:boolean);
begin
if global_scope then write(umber_trace_file,'"global"')
else write(umber_trace_file,'"local"');
end;

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

procedure umber_trace_glue_value(@!p:pointer);
begin
write(umber_trace_file,'{"type":"glue","value":{"width":',width(p):1,
  ',"stretch":',stretch(p):1,',"stretch_order":');
umber_trace_glue_order(stretch_order(p));
write(umber_trace_file,',"shrink":',shrink(p):1,',"shrink_order":');
umber_trace_glue_order(shrink_order(p));
write(umber_trace_file,'}}');
end;

procedure umber_trace_meaning_value(@!t:quarterword;@!e:halfword);
begin
if (t>=call)and(e<>null) then
  begin write(umber_trace_file,'{"type":"tokens","value":[');
  umber_trace_token_range(link(e),null);
  write(umber_trace_file,']}');
  end
else if t=char_given then
  write(umber_trace_file,'{"type":"character","value":',e:1,'}')
else if t=math_given then
  write(umber_trace_file,'{"type":"integer","value":',e:1,'}')
else begin
  write(umber_trace_file,'{"type":"name","value":');
  umber_trace_command_name(t);
  write(umber_trace_file,'}');
  end;
end;

procedure umber_trace_eq_mutation(@!p:pointer;@!t:quarterword;
  @!e:halfword;@!global_scope:boolean);
var family,@!slot:integer;
begin
if (not umber_trace_opened)or(not umber_mutation_command) then return;
if p<glue_base then
  begin
  if p>=undefined_control_sequence then return;
  umber_trace_begin;
  write(umber_trace_file,'{"event":"mutation","data":{"target":"meaning",');
  write(umber_trace_file,'"key":{"type":"name","value":');
  umber_trace_cs(p); write(umber_trace_file,'},"value":');
  umber_trace_meaning_value(t,e);
  write(umber_trace_file,',"scope":'); umber_trace_scope(global_scope);
  write_ln(umber_trace_file,'}}}');
  return;
  end;
family:=-1; slot:=0;
if p<skip_base then begin family:=0; slot:=p-glue_base; end
else if p<mu_skip_base then begin family:=1; slot:=p-skip_base; end
else if p<local_base then begin family:=2; slot:=p-mu_skip_base; end
else if (p>=output_routine_loc)and(p<toks_base) then
  begin family:=3; slot:=p-output_routine_loc; end
else if (p>=toks_base)and(p<box_base) then
  begin family:=4; slot:=p-toks_base; end
else if (p>=cat_code_base)and(p<lc_code_base) then
  begin
  umber_trace_begin;
  write(umber_trace_file,'{"event":"mutation","data":{"target":"catcode",');
  write(umber_trace_file,'"key":{"type":"character","value":',
    p-cat_code_base:1,'},"value":{"type":"name","value":');
  umber_trace_catcode(e);
  write(umber_trace_file,'},"scope":'); umber_trace_scope(global_scope);
  write_ln(umber_trace_file,'}}}');
  return;
  end
else if (p>=lc_code_base)and(p<uc_code_base) then
  begin family:=5; slot:=p-lc_code_base; end
else if (p>=uc_code_base)and(p<sf_code_base) then
  begin family:=6; slot:=p-uc_code_base; end
else if (p>=sf_code_base)and(p<math_code_base) then
  begin family:=7; slot:=p-sf_code_base; end
else if (p>=math_code_base)and(p<int_base) then
  begin family:=8; slot:=p-math_code_base; end;
if family<0 then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"mutation","data":{"target":"');
if family<=4 then
  if (family=0)or(family=3) then write(umber_trace_file,'parameter')
  else write(umber_trace_file,'register')
else write(umber_trace_file,'code_table');
write(umber_trace_file,'","key":{"type":"name","value":');
umber_trace_named_slot(family,slot);
write(umber_trace_file,'},"value":');
if family<=2 then umber_trace_glue_value(e)
else if family<=4 then
  begin write(umber_trace_file,'{"type":"tokens","value":[');
  if e<>null then umber_trace_token_range(link(e),null);
  write(umber_trace_file,']}');
  end
else write(umber_trace_file,'{"type":"integer","value":',e:1,'}');
write(umber_trace_file,',"scope":'); umber_trace_scope(global_scope);
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_word_mutation(@!p:pointer;@!w:integer;
  @!global_scope:boolean);
var family,@!slot:integer;
begin
if (not umber_trace_opened)or(not umber_mutation_command) then return;
family:=-1; slot:=0;
if (p>=int_base)and(p<count_base) then
  begin family:=9; slot:=p-int_base; end
else if (p>=count_base)and(p<del_code_base) then
  begin family:=10; slot:=p-count_base; end
else if (p>=del_code_base)and(p<dimen_base) then
  begin family:=11; slot:=p-del_code_base; end
else if (p>=dimen_base)and(p<scaled_base) then
  begin family:=12; slot:=p-dimen_base; end
else if (p>=scaled_base)and(p<=eqtb_size) then
  begin family:=13; slot:=p-scaled_base; end;
if family<0 then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"mutation","data":{"target":"');
if (family=9)or(family=12) then write(umber_trace_file,'parameter')
else if family=11 then write(umber_trace_file,'code_table')
else write(umber_trace_file,'register');
write(umber_trace_file,'","key":{"type":"name","value":');
umber_trace_named_slot(family,slot);
write(umber_trace_file,'},"value":{"type":"');
if family>=12 then write(umber_trace_file,'scaled')
else write(umber_trace_file,'integer');
write(umber_trace_file,'","value":',w:1,'},"scope":');
umber_trace_scope(global_scope);
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_pool_bytes(@!s:str_number);
var k:pool_pointer; first_byte:boolean;
begin
write(umber_trace_file,'{"type":"bytes","value":[');
first_byte:=true;
for k:=str_start[s] to str_start[s+1]-1 do
  begin
  if first_byte then first_byte:=false else write(umber_trace_file,',');
  write(umber_trace_file,so(str_pool[k]):1);
  end;
write(umber_trace_file,']}');
end;

procedure umber_trace_file_name_value;
begin
write(umber_trace_file,'{"type":"name","value":"');
umber_trace_string_contents(cur_area);
umber_trace_string_contents(cur_name);
umber_trace_string_contents(cur_ext);
write(umber_trace_file,'"}');
end;

procedure umber_trace_effect(@!effect_kind:integer;@!channel:integer;
  @!value_kind:integer;@!value:integer);
begin
if not umber_trace_opened then return;
umber_trace_begin;
write(umber_trace_file,'{"event":"effect","data":{"kind":"');
case effect_kind of
0:write(umber_trace_file,'message');
1:write(umber_trace_file,'write');
2:write(umber_trace_file,'open');
3:write(umber_trace_file,'close');
othercases write(umber_trace_file,'shipout')
endcases;
write(umber_trace_file,'","channel":"');
if effect_kind=0 then write(umber_trace_file,'terminal')
else if effect_kind=4 then write(umber_trace_file,'dvi')
else write(umber_trace_file,'stream:',channel:1);
write(umber_trace_file,'","value":');
if value_kind=0 then umber_trace_pool_bytes(value)
else if value_kind=1 then
  begin write(umber_trace_file,'{"type":"tokens","value":[');
  if value<>null then umber_trace_token_range(link(value),null);
  write(umber_trace_file,']}');
  end
else if value_kind=2 then
  write(umber_trace_file,'{"type":"integer","value":',value:1,'}')
else if value_kind=3 then umber_trace_file_name_value
else write(umber_trace_file,'{"type":"none"}');
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_open;
begin umber_trace_sequence:=0; umber_recovery_insert:=false;
umber_alignment_depth:=0; umber_mutation_command:=false;
umber_line_shift:=0; umber_shift_tail:=0; umber_shift_pos:=0;
  rewrite(umber_trace_file,'pdftex14029-events.jsonl');
umber_trace_opened:=true;
if umber_trace_opened then write_ln(umber_trace_file,
 '{"schema":1,"manifest":',
 '"0000000000000000000000000000000000000000000000000000000000000000"}');
umber_diag_sequence:=0; umber_diag_has_report:=false;
umber_diag_has_location:=false;
rewrite(umber_diag_file,'pdftex14029-diagnostics.jsonl');
umber_diag_opened:=true;
if umber_diag_opened then write_ln(umber_diag_file,
 '{"schema":4,"manifest":',
 '"0000000000000000000000000000000000000000000000000000000000000000"}');
end;

procedure umber_trace_finish;
begin
if not umber_trace_opened then return;
umber_trace_input(2,0,0);
umber_trace_begin;
write_ln(umber_trace_file,
 '{"event":"effect","data":{"kind":"terminate","channel":"engine",',
 '"value":{"type":"none"}}}}');
a_close(umber_trace_file); umber_trace_opened:=false;
if umber_diag_opened then
  begin
  if umber_diag_has_report then
    begin
    umber_diag_begin;
    write(umber_diag_file,'{"event":"diagnostic_lifecycle",');
    write(umber_diag_file,'"data":{"transition":"outcome","history":"');
    if history=spotless then write(umber_diag_file,'spotless')
    else if history=warning_issued then write(umber_diag_file,'warning_issued')
    else if history=error_message_issued then write(umber_diag_file,'error_message_issued')
    else write(umber_diag_file,'fatal_error_stop');
    if history=fatal_error_stop then
      write_ln(umber_diag_file,'","outcome":"aborted"}}}')
    else write_ln(umber_diag_file,'","outcome":"completed"}}}');
    end;
  a_close(umber_diag_file); umber_diag_opened:=false;
  end;
end;
@z

@x [18] Observe committed local equivalent writes at the pdfTeX seam.
eq_level(p):=cur_level; eq_type(p):=t; equiv(p):=e;
assign_trace(p,"into")@;@/
exit:end;
@y
eq_level(p):=cur_level; eq_type(p):=t; equiv(p):=e;
umber_trace_eq_mutation(p,t,e,false);
assign_trace(p,"into")@;@/
exit:end;
@z

@x [18] Observe committed local fullword writes at the pdfTeX seam.
eqtb[p].int:=w;
assign_trace(p,"into")@;@/
exit:end;
@y
eqtb[p].int:=w;
umber_trace_word_mutation(p,w,false);
assign_trace(p,"into")@;@/
exit:end;
@z

@x [18] Observe committed global equivalent writes.
eq_level(p):=level_one; eq_type(p):=t; equiv(p):=e;
end;
@y
eq_level(p):=level_one; eq_type(p):=t; equiv(p):=e;
umber_trace_eq_mutation(p,t,e,true);
end;
@z

@x [18] Observe committed global fullword writes.
begin eqtb[p].int:=w; xeq_level[p]:=level_one;
end;
@y
begin eqtb[p].int:=w; xeq_level[p]:=level_one;
umber_trace_word_mutation(p,w,true);
end;
@z

@x [23] Observe token-list pushes.
else loc:=p;
end;
@y
else loc:=p;
if t=backed_up then
  if umber_recovery_insert then umber_trace_input(0,5,t)
  else umber_trace_input(0,4,t)
else if t=inserted then
  begin umber_trace_input(0,5,t);
  if p<>null then
    if info(p)>=cs_token_flag then umber_trace_recovery(2,info(p))
    else umber_trace_recovery(1,info(p));
  end
else if t=macro then umber_trace_input(0,2,t)
else if (t=u_template)or(t=v_template) then
  begin umber_trace_input(0,3,t);
  if t=u_template then umber_trace_alignment(7,align_state,1)
  else if p=omit_template then umber_trace_alignment(7,align_state,3)
  else umber_trace_alignment(7,align_state,2);
  end
else umber_trace_input(0,1,t);
end;
@z

@x [23] Observe token-list retirement.
@p procedure end_token_list; {leave a token-list input level}
begin if token_type>=backed_up then {token list to be deleted}
@y
@p procedure end_token_list; {leave a token-list input level}
begin
if token_type=backed_up then umber_trace_input(1,4,token_type)
else if token_type=inserted then umber_trace_input(1,5,token_type)
else if token_type=macro then umber_trace_input(1,2,token_type)
else if (token_type=u_template)or(token_type=v_template) then
  begin umber_trace_input(1,3,token_type);
  if token_type=u_template then umber_trace_alignment(8,align_state,1)
  else if start=omit_template then umber_trace_alignment(8,align_state,3)
  else umber_trace_alignment(8,align_state,2);
  end
else umber_trace_input(1,1,token_type);
if token_type>=backed_up then {token list to be deleted}
@z

@x [23] Observe u-template completion at the canonical retirement seam.
else if token_type=u_template then
  if align_state>500000 then align_state:=0
  else fatal_error("(interwoven alignment preambles are not allowed)");
@y
else if token_type=u_template then
  if align_state>500000 then
    begin align_state:=0; umber_trace_alignment(6,1000000,0); end
  else fatal_error("(interwoven alignment preambles are not allowed)");
@z

@x [23] Observe exact backup/recovery after commit.
push_input; state:=token_list; start:=p; token_type:=backed_up;
loc:=p; {that was |back_list(p)|, without procedure overhead}
end;
@y
push_input; state:=token_list; start:=p;
if umber_recovery_insert then token_type:=inserted
else token_type:=backed_up;
loc:=p; {that was |back_list(p)|, without procedure overhead}
if umber_recovery_insert then
  begin umber_trace_input(0,5,token_type);
  umber_trace_recovery(1,cur_tok); end
else begin umber_trace_input(0,4,token_type);
  umber_trace_recovery(0,cur_tok); end;
if cur_tok<right_brace_limit then
  if cur_tok<left_brace_limit then
    umber_trace_alignment(9,align_state+1,0)
  else umber_trace_alignment(9,align_state-1,0);
end;
@z

@x [23] Distinguish inserted recovery.
procedure ins_error; {back up one inserted token and call |error|}
begin OK_to_interrupt:=false; back_input; token_type:=inserted;
OK_to_interrupt:=true; error;
end;
@y
procedure ins_error; {back up one inserted token and call |error|}
begin OK_to_interrupt:=false; umber_recovery_insert:=true;
back_input; umber_recovery_insert:=false;
OK_to_interrupt:=true; error;
end;
@z

@x [23] Observe source pushes after pdfTeX/Web2C source initialization.
name:=0; {|terminal_input| is now |true|}
@<Prepare terminal input {\sl Sync\TeX} information@>;
end;
@y
name:=0; {|terminal_input| is now |true|}
umber_shift_stack[index]:=umber_line_shift;
umber_tail_stack[index]:=umber_shift_tail;
umber_pos_stack[index]:=umber_shift_pos;
umber_line_shift:=0; umber_shift_tail:=0; umber_shift_pos:=0;
umber_trace_input(0,0,0);
@<Prepare terminal input {\sl Sync\TeX} information@>;
end;
@z

@x [23] Observe pdfTeX pseudo-file and ordinary source retirement.
@p procedure end_file_reading;
begin first:=start; line:=line_stack[index];
if (name=18)or(name=19) then pseudo_close else
if name>17 then a_close(cur_file); {forget it}
pop_input; decr(in_open);
end;
@y
@p procedure end_file_reading;
begin umber_trace_input(1,0,0);
first:=start; line:=line_stack[index];
umber_line_shift:=umber_shift_stack[index];
umber_shift_tail:=umber_tail_stack[index];
umber_shift_pos:=umber_pos_stack[index];
if (name=18)or(name=19) then pseudo_close else
if name>17 then a_close(cur_file); {forget it}
pop_input; decr(in_open);
end;
@z

@x [24] Observe outer validity before recovery.
begin if scanner_status<>normal then
  begin deletions_allowed:=false;
@y
begin if scanner_status<>normal then
  begin umber_trace_outer(cur_cs=0);
  if scanner_status=aligning then umber_trace_alignment_recovery(5);
  if scanner_status=skipping then umber_trace_conditional_diagnostic(0);
  deletions_allowed:=false;
@z

@x [24] Observe outer control-sequence backup.
    back_list(p); {prepare to read the control sequence again}
    end;
@y
    back_list(p); {prepare to read the control sequence again}
    umber_trace_recovery(0,cs_token_flag+cur_cs);
    end;
@z

@x [24] Raw delivery commits at get_next exit.
@<If an alignment entry has just ended, take appropriate action@>;
exit:end;
@y
@<If an alignment entry has just ended, take appropriate action@>;
exit:umber_trace_command(false);
end;
@z

@x [24] Observe source-token brace accounting.
mid_line+left_brace: incr(align_state);
@y
mid_line+left_brace: begin incr(align_state);
  umber_trace_alignment(6,align_state-1,0); end;
@z

@x [24] Observe source-token brace accounting after line-state changes.
skip_blanks+left_brace,new_line+left_brace: begin
  state:=mid_line; incr(align_state);
  end;
@y
skip_blanks+left_brace,new_line+left_brace: begin
  state:=mid_line; incr(align_state);
  umber_trace_alignment(6,align_state-1,0);
  end;
@z

@x [24] Observe source-token right-brace accounting.
mid_line+right_brace: decr(align_state);
@y
mid_line+right_brace: begin decr(align_state);
  umber_trace_alignment(6,align_state+1,0);
  end;
@z

@x [24] Observe source-token right-brace accounting after state changes.
skip_blanks+right_brace,new_line+right_brace: begin
  state:=mid_line; decr(align_state);
  end;
@y
skip_blanks+right_brace,new_line+right_brace: begin
  state:=mid_line; decr(align_state);
  umber_trace_alignment(6,align_state+1,0);
  end;
@z

@x [24] Record the source-column effect before \S355 shifts the buffer.
    else if c<@'100 then buffer[k-1]:=c+@'100
    else buffer[k-1]:=c-@'100;
@y
    else if c<@'100 then buffer[k-1]:=c+@'100
    else buffer[k-1]:=c-@'100;
    umber_line_shift:=umber_line_shift+d; umber_shift_tail:=d;
    umber_shift_pos:=k-1-start;
@z

@x [24] Observe token-list brace accounting.
    case cur_cmd of
    left_brace: incr(align_state);
    right_brace: decr(align_state);
@y
    case cur_cmd of
    left_brace: begin incr(align_state);
      umber_trace_alignment(6,align_state-1,0); end;
    right_brace: begin decr(align_state);
      umber_trace_alignment(6,align_state+1,0); end;
@z

@x [24] Terminal read stop is a raw zero command.
    begin cur_cmd:=0; cur_chr:=0; return;
    end;
@y
    begin cur_cmd:=0; cur_chr:=0; umber_trace_command(false);
    umber_trace_input(2,0,0); return;
    end;
@z

@x [24] A freshly read line has collapsed no bytes yet.
first:=limit+1; loc:=start; {ready to read}
end
@y
first:=limit+1; loc:=start; {ready to read}
umber_line_shift:=0; umber_shift_tail:=0; umber_shift_pos:=0;
end
@z

@x [25] noexpand scanner status.
begin save_scanner_status:=scanner_status; scanner_status:=normal;
get_token; scanner_status:=save_scanner_status; t:=cur_tok;
@y
begin save_scanner_status:=scanner_status; umber_set_scanner_status(normal);
get_token; umber_set_scanner_status(save_scanner_status); t:=cur_tok;
@z

@x [25] Observe canonical inserted-relax recovery.
procedure insert_relax;
begin cur_tok:=cs_token_flag+cur_cs; back_input;
cur_tok:=cs_token_flag+frozen_relax; back_input; token_type:=inserted;
end;
@y
procedure insert_relax;
begin cur_tok:=cs_token_flag+cur_cs; back_input;
cur_tok:=cs_token_flag+frozen_relax; umber_recovery_insert:=true;
back_input; umber_recovery_insert:=false;
if cur_cmd=fi_or_else then umber_trace_conditional_diagnostic(1);
end;
@z

@x [25] Expanded delivery from get_x_token.
done: if cur_cs=0 then cur_tok:=(cur_cmd*@'400)+cur_chr
else cur_tok:=cs_token_flag+cur_cs;
end;
@y
done: if cur_cs=0 then cur_tok:=(cur_cmd*@'400)+cur_chr
else cur_tok:=cs_token_flag+cur_cs;
umber_trace_command(true);
end;
@z

@x [25] Expanded delivery from x_token.
if cur_cs=0 then cur_tok:=(cur_cmd*@'400)+cur_chr
else cur_tok:=cs_token_flag+cur_cs;
end;
@y
if cur_cs=0 then cur_tok:=(cur_cmd*@'400)+cur_chr
else cur_tok:=cs_token_flag+cur_cs;
umber_trace_command(true);
end;
@z

@x [25] Macro matching restoration.
exit:scanner_status:=save_scanner_status; warning_index:=save_warning_index;
@y
exit:umber_set_scanner_status(save_scanner_status);
warning_index:=save_warning_index;
@z

@x [25] Observe committed macro activation.
  for m:=0 to n-1 do param_stack[param_ptr+m]:=pstack[m];
  param_ptr:=param_ptr+n;
  end
@y
  for m:=0 to n-1 do param_stack[param_ptr+m]:=pstack[m];
  param_ptr:=param_ptr+n;
  end;
umber_trace_macro_activation(warning_index,n)
@z

@x [25] Macro matching status.
begin scanner_status:=matching; unbalance:=0;
@y
begin umber_set_scanner_status(matching); unbalance:=0;
@z

@x [25] Observe a completely matched macro delimiter.
found: if s<>null then @<Tidy up the parameter just scanned, and tuck it away@>
@y
found: if s<>null then
  begin umber_trace_token_list(0,0,s,r);
  @<Tidy up the parameter just scanned, and tuck it away@>;
  end
@z

@x [25] Observe committed overlapping-delimiter recovery.
    repeat store_new_token(info(t)); incr(m); u:=link(t); v:=s;
@y
    repeat store_new_token(info(t)); incr(m);
    umber_trace_token_splice(1,info(t)); u:=link(t); v:=s;
@z

@x [25] Observe each completed macro argument after brace stripping.
else pstack[n]:=link(temp_head);
incr(n);
@y
else pstack[n]:=link(temp_head);
umber_trace_macro_argument(n+1,pstack[n]);
incr(n);
@z

@x [26] Observe committed internal scanner results.
@<Fix the reference count, if any, and negate |cur_val| if |negative|@>;
exit:end;
@y
@<Fix the reference count, if any, and negate |cur_val| if |negative|@>;
umber_trace_scanner(3,cur_val_level);
exit:end;
@z

@x [26] Observe token-list-as-number recovery.
if level<>tok_val then
  begin print_err("Missing number, treated as zero");
@.Missing number...@>
  help3("A number should have been here; I inserted `0'.")@/
@y
if level<>tok_val then
  begin print_err("Missing number, treated as zero");
  umber_diag_report(0,"missing-number",0);
@.Missing number...@>
  help3("A number should have been here; I inserted `0'.")@/
@z

@x [26] Observe committed integer scanner results.
if negative then negate(cur_val);
end;
@y
if negative then negate(cur_val);
umber_trace_scanner(0,0);
end;
@z

@x [26] Observe generic missing-number recovery.
begin print_err("Missing number, treated as zero");
@.Missing number...@>
help3("A number should have been here; I inserted `0'.")@/
@y
begin print_err("Missing number, treated as zero");
umber_diag_report(0,"missing-number",0);
@.Missing number...@>
help3("A number should have been here; I inserted `0'.")@/
@z

@x [26] Observe committed dimension scanner results.
if negative then negate(cur_val);
end;
@y
if negative then negate(cur_val);
umber_trace_scanner(1,1);
end;
@z

@x [26] Observe ordinary-dimension illegal-unit recovery.
begin print_err("Illegal unit of measure ("); print("pt inserted)");
@.Illegal unit of measure@>
help6("Dimensions can be in units of em, ex, in, pt, pc,")@/
@y
begin print_err("Illegal unit of measure ("); print("pt inserted)");
umber_diag_report(0,"illegal-unit","pt");
@.Illegal unit of measure@>
help6("Dimensions can be in units of em, ex, in, pt, pc,")@/
@z

@x [26] Observe an internal glue result before the early return.
    begin if cur_val_level<>level then mu_error;
    return;
    end;
@y
    begin if cur_val_level<>level then mu_error;
    umber_trace_scanner(2,cur_val_level);
    return;
    end;
@z

@x [26] Observe newly constructed glue after all components commit.
exit:end;
@y
exit:umber_trace_scanner(2,level);
end;
@z

@x [27] string/meaning scanner status.
string_code, meaning_code: begin save_scanner_status:=scanner_status;
  scanner_status:=normal; get_token; scanner_status:=save_scanner_status;
@y
string_code, meaning_code: begin save_scanner_status:=scanner_status;
  umber_set_scanner_status(normal); get_token;
  umber_set_scanner_status(save_scanner_status);
@z

@x [27] Retain the canonical replacement-list boundary for observation.
@!q:pointer; {new node being added to the token list via |store_new_token|}
@!unbalance:halfword; {number of unmatched left braces}
@y
@!q:pointer; {new node being added to the token list via |store_new_token|}
@!umber_trace_start:pointer; {predecessor of the collected replacement}
@!unbalance:halfword; {number of unmatched left braces}
@z

@x [27] Balanced-text status entry.
begin if macro_def then scanner_status:=defining
@+else scanner_status:=absorbing;
@y
begin if macro_def then umber_set_scanner_status(defining)
@+else umber_set_scanner_status(absorbing);
umber_trace_start:=null;
@z

@x [27] Mark the beginning of collected replacement text.
if macro_def then @<Scan and build the parameter part of the macro definition@>
else scan_left_brace; {remove the compulsory left brace}
@<Scan and build the body of the token list; |goto found| when finished@>;
@y
if macro_def then @<Scan and build the parameter part of the macro definition@>
else scan_left_brace; {remove the compulsory left brace}
umber_trace_start:=p;
@<Scan and build the body of the token list; |goto found| when finished@>;
@z

@x [27] Observe completed scan_toks collection.
found: scanner_status:=normal;
if hash_brace<>0 then store_new_token(hash_brace);
scan_toks:=p;
@y
found: umber_set_scanner_status(normal);
if hash_brace<>0 then store_new_token(hash_brace);
{|umber_trace_start| stays |null| when the collection never reached its
replacement text: section 475's missing-left-brace recovery does |goto found|
from inside the parameter scan, and a runaway definition ends the same way.
There is no completed replacement list to observe on those paths, and the
local is otherwise uninitialized, so |link| of it would read outside |mem|.}
if umber_trace_start<>null then
  if macro_def then
    umber_trace_token_list(1,3,link(umber_trace_start),null)
  else if xpand then
    umber_trace_token_list(1,5,link(umber_trace_start),null)
  else umber_trace_token_list(1,4,link(umber_trace_start),null);
scan_toks:=p;
@z

@x [27] Observe protected-macro suppression in expanded token construction.
  if cur_cmd>=call then
    if info(link(cur_chr))=protected_token then
      begin cur_cmd:=relax; cur_chr:=no_expand_flag;
      end;
@y
  if cur_cmd>=call then
    if info(link(cur_chr))=protected_token then
      begin umber_trace_token_splice(11,cs_token_flag+cur_cs);
      cur_cmd:=relax; cur_chr:=no_expand_flag;
      end;
@z

@x [27] Observe direct token-list splices made by \the.
  if cur_cmd<=max_command then goto done2;
  if cur_cmd<>the then expand
  else  begin q:=the_toks;
    if link(temp_head)<>null then
      begin link(p):=link(temp_head); p:=q;
      end;
    end;
@y
  if cur_cmd<=max_command then goto done2;
  if cur_cmd<>the then expand
  else  begin q:=the_toks;
    if link(temp_head)<>null then
      begin umber_trace_token_list(0,6,link(temp_head),null);
      link(p):=link(temp_head); p:=q;
      end;
    end;
@z

@x [27] Observe conversion from parameter spelling to replay token.
  else cur_tok:=out_param_token-"0"+cur_chr;
@y
  else begin cur_tok:=out_param_token-"0"+cur_chr;
    umber_trace_token_splice(2,cur_tok);
    end;
@z

@x [27] read_toks status entry.
begin scanner_status:=defining; warning_index:=r;
@y
begin umber_set_scanner_status(defining); warning_index:=r;
@z

@x [27] read_toks status restoration.
cur_val:=def_ref; scanner_status:=normal; align_state:=s;
@y
cur_val:=def_ref; umber_set_scanner_status(normal); align_state:=s;
@z

@x [28] Conditional skipping status.
begin save_scanner_status:=scanner_status; scanner_status:=skipping; l:=0;
@y
begin save_scanner_status:=scanner_status;
umber_set_scanner_status(skipping); l:=0;
@z

@x [28] Conditional skipping restoration.
done: scanner_status:=save_scanner_status;
@y
done: umber_set_scanner_status(save_scanner_status);
if cur_chr=or_code then umber_trace_condition(2,cur_if,if_limit,3)
else if cur_chr=else_code then umber_trace_condition(2,cur_if,if_limit,4)
else umber_trace_condition(2,cur_if,if_limit,5);
@z

@x [28] Observe condition-frame push after it commits.
cond_ptr:=p; cur_if:=cur_chr; if_limit:=if_code; if_line:=line;
end
@y
cond_ptr:=p; cur_if:=cur_chr; if_limit:=if_code; if_line:=line;
umber_trace_condition(0,cur_if,if_limit,0);
end
@z

@x [28] Observe condition-frame pop before its storage is retired.
begin if if_stack[in_open]=cond_ptr then if_warning;
  {conditionals possibly not properly nested with files}
p:=cond_ptr; if_line:=if_line_field(p);
cur_if:=subtype(p); if_limit:=type(p); cond_ptr:=link(p);
free_node(p,if_node_size);
end
@y
begin if if_stack[in_open]=cond_ptr then if_warning;
  {conditionals possibly not properly nested with files}
p:=cond_ptr;
umber_trace_condition(3,cur_if,if_limit,0);
if_line:=if_line_field(p);
cur_if:=subtype(p); if_limit:=type(p); cond_ptr:=link(p);
free_node(p,if_node_size);
end
@z

@x [28] Observe exact-frame conditional limit changes.
begin if p=cond_ptr then if_limit:=l {that's the easy case}
else  begin q:=cond_ptr;
  loop@+  begin if q=null then confusion("if");
@:this can't happen if}{\quad if@>
    if link(q)=p then
      begin type(q):=l; return;
      end;
    q:=link(q);
    end;
  end;
exit:end;
@y
begin if p=cond_ptr then
  begin if_limit:=l; umber_trace_condition(1,cur_if,l,0); end
else  begin q:=cond_ptr;
  loop@+  begin if q=null then confusion("if");
@:this can't happen if}{\quad if@>
    if link(q)=p then
      begin type(q):=l; umber_trace_condition(1,subtype(q),l,0); return;
      end;
    q:=link(q);
    end;
  end;
exit:end;
@z

@x [28] Observe the selected boolean branch.
if tracing_commands>1 then @<Display the value of |b|@>;
if b then
@y
if tracing_commands>1 then @<Display the value of |b|@>;
if b then umber_trace_condition(2,this_if,if_limit,1)
else umber_trace_condition(2,this_if,if_limit,2);
if b then
@z

@x [28] Observe the direct false-branch delimiter change.
common_ending: if cur_chr=fi_code then @<Pop the condition stack@>
else if_limit:=fi_code; {wait for \.{\\fi}}
@y
common_ending: if cur_chr=fi_code then @<Pop the condition stack@>
else begin if_limit:=fi_code;
  umber_trace_condition(1,cur_if,if_limit,0);
  end; {wait for \.{\\fi}}
@z

@x [28] Conditional operand status entry.
begin save_scanner_status:=scanner_status; scanner_status:=normal;
@y
begin save_scanner_status:=scanner_status; umber_set_scanner_status(normal);
@z

@x [28] Conditional operand status restoration.
scanner_status:=save_scanner_status;
@y
umber_set_scanner_status(save_scanner_status);
@z

@x [28] Observe selected ifcase branch after skip progress.
change_if_limit(or_code,save_cond_ptr);
return; {wait for \.{\\or}, \.{\\else}, or \.{\\fi}}
@y
change_if_limit(or_code,save_cond_ptr);
umber_trace_condition(2,this_if,or_code,6);
return; {wait for \.{\\or}, \.{\\else}, or \.{\\fi}}
@z

@x [32] Observe successful shipout after the page commits.
dvi_out(eop); incr(total_pages); cur_s:=-1;
@y
dvi_out(eop); incr(total_pages);
umber_trace_effect(4,0,2,total_pages);
cur_s:=-1;
@z

@x [45] Observe nested alignment suspension and ownership.
align_ptr:=p;
cur_head:=get_avail;
cur_pre_head:=get_avail;
end;
@y
align_ptr:=p;
cur_head:=get_avail;
cur_pre_head:=get_avail;
if umber_alignment_depth>0 then
  umber_trace_alignment(2,align_state,0);
incr(umber_alignment_depth);
end;
@z

@x [45] Observe alignment retirement and exact outer-state restoration.
cur_tail:=link(p+4); cur_head:=info(p+4);
cur_pre_tail:=link(p+5); cur_pre_head:=info(p+5);
align_state:=mem[p+3].int; cur_loop:=mem[p+2].int;
@y
umber_trace_alignment(1,align_state,0);
cur_tail:=link(p+4); cur_head:=info(p+4);
cur_pre_tail:=link(p+5); cur_pre_head:=info(p+5);
align_state:=mem[p+3].int; cur_loop:=mem[p+2].int;
decr(umber_alignment_depth);
if umber_alignment_depth>0 then
  umber_trace_alignment(3,align_state,0);
@z

@x [45] Observe alignment entry state.
push_alignment; align_state:=-1000000; {enter a new alignment level}
@y
push_alignment; align_state:=-1000000; {enter a new alignment level}
umber_trace_alignment(0,-1000000,0);
@z

@x [45] Alignment status entry.
preamble:=null; cur_align:=align_head; cur_loop:=null; scanner_status:=aligning;
@y
preamble:=null; cur_align:=align_head; cur_loop:=null;
umber_set_scanner_status(aligning);
@z

@x [45] Observe preamble lifecycle entry.
warning_index:=save_cs_ptr; align_state:=-1000000;
@y
warning_index:=save_cs_ptr; align_state:=-1000000;
umber_trace_alignment(4,-1000000,0);
@z

@x [45] Alignment status restoration.
done: scanner_status:=normal
@y
done: umber_trace_alignment(5,align_state,0);
umber_set_scanner_status(normal)
@z

@x [45] Observe missing-parameter preamble recovery.
    back_error; goto done1;
@y
    umber_trace_alignment_recovery(0);
    back_error; goto done1;
@z

@x [45] Observe extra-parameter preamble recovery.
    error; goto continue;
@y
    umber_trace_alignment_recovery(1);
    error; goto continue;
@z

@x [45] Observe lookahead state installation.
begin restart: align_state:=1000000;
repeat get_x_or_protected;
until cur_cmd<>spacer;
@y
begin restart: align_state:=1000000;
umber_trace_alignment(6,align_state,0);
repeat get_x_or_protected;
until cur_cmd<>spacer;
@z

@x [45] Observe u-template or omit cell-state installation.
if cur_cmd=omit then align_state:=0
else  begin back_input; begin_token_list(u_part(cur_align),u_template);
@y
if cur_cmd=omit then
  begin align_state:=0; umber_trace_alignment(6,1000000,0); end
else  begin back_input; begin_token_list(u_part(cur_align),u_template);
@z

@x [45] Observe delimiter interception and v-template state installation.
begin if (scanner_status=aligning) or (cur_align=null) then
  fatal_error("(interwoven alignment preambles are not allowed)");
@.interwoven alignment preambles...@>
cur_cmd:=extra_info(cur_align); extra_info(cur_align):=cur_chr;
if cur_cmd=omit then begin_token_list(omit_template,v_template)
else begin_token_list(v_part(cur_align),v_template);
align_state:=1000000; goto restart;
end
@y
begin if (scanner_status=aligning) or (cur_align=null) then
  fatal_error("(interwoven alignment preambles are not allowed)");
@.interwoven alignment preambles...@>
umber_trace_alignment_delimiter(cur_cmd,cur_chr);
cur_cmd:=extra_info(cur_align); extra_info(cur_align):=cur_chr;
if cur_cmd=omit then begin_token_list(omit_template,v_template)
else begin_token_list(v_part(cur_align),v_template);
align_state:=1000000; umber_trace_alignment(6,0,0); goto restart;
end
@z

@x [45] Observe fin_col lookahead state installation.
align_state:=1000000;
repeat get_x_or_protected;
until cur_cmd<>spacer;
@y
align_state:=1000000; umber_trace_alignment(6,align_state,0);
repeat get_x_or_protected;
until cur_cmd<>spacer;
@z

@x [45] Observe extra-tab alignment recovery.
  extra_info(cur_align):=cr_code; error;
@y
  extra_info(cur_align):=cr_code;
  umber_trace_alignment_recovery(4); error;
@z

@x [40] Observe |off_save| recovery versus bounded bottom-level dropping.
procedure off_save;
var p:pointer; {inserted token}
begin if cur_group=bottom_level then
  @<Drop current token and complain that it was unmatched@>
else  begin back_input; p:=get_avail; link(temp_head):=p;
@y
procedure off_save;
var p:pointer; {inserted token}
begin if cur_group=bottom_level then
  begin umber_trace_off_save(true,cur_tok);
  @<Drop current token and complain that it was unmatched@>
  end
else  begin umber_trace_off_save(false,cur_tok);
  back_input; p:=get_avail; link(temp_head):=p;
@z

@x [49] Observe missing-brace alignment recovery.
  if align_state<0 then
    begin print_err("Missing { inserted");
@.Missing \{ inserted@>
    incr(align_state); cur_tok:=left_brace_token+"{";
    end
  else  begin print_err("Missing } inserted");
@.Missing \} inserted@>
    decr(align_state); cur_tok:=right_brace_token+"}";
    end;
@y
  if align_state<0 then
    begin print_err("Missing { inserted");
@.Missing \{ inserted@>
    umber_trace_alignment_recovery(2);
    incr(align_state); umber_trace_alignment(6,align_state-1,0);
    cur_tok:=left_brace_token+"{";
    end
  else  begin print_err("Missing } inserted");
@.Missing \} inserted@>
    umber_trace_alignment_recovery(3);
    decr(align_state); umber_trace_alignment(6,align_state+1,0);
    cur_tok:=right_brace_token+"}";
end;
@z

@x [49] Scope committed eqtb mutations to assignment commands.
@<Adjust \(f)for the setting of \.{\\globaldefs}@>;
case cur_cmd of
@t\4@>@<Assignments@>@;
othercases confusion("prefix")
@:this can't happen prefix}{\quad prefix@>
endcases;
done: @<Insert a token saved by \.{\\afterassignment}, if any@>;
@y
@<Adjust \(f)for the setting of \.{\\globaldefs}@>;
umber_mutation_command:=true;
case cur_cmd of
@t\4@>@<Assignments@>@;
othercases confusion("prefix")
@:this can't happen prefix}{\quad prefix@>
endcases;
done: umber_mutation_command:=false;
@<Insert a token saved by \.{\\afterassignment}, if any@>;
@z

@x [49] Observe the semantic protected-macro definition after flag insertion.
  if j<>0 then
    begin q:=get_avail; info(q):=j; link(q):=link(def_ref);
    link(def_ref):=q;
    end;
  define(p,call+(a mod 4),def_ref);
@y
  if j<>0 then
    begin q:=get_avail; info(q):=j; link(q):=link(def_ref);
    link(def_ref):=q;
    umber_trace_token_list(1,10,link(q),null);
    end;
  define(p,call+(a mod 4),def_ref);
@z

@x [50] Observe an ordinary message after it is printed.
if c=0 then @<Print string |s| on the terminal@>
else @<Print string |s| as an error message@>;
flush_string;
@y
if c=0 then
  begin @<Print string |s| on the terminal@>;
  umber_trace_effect(0,0,0,s);
  end
else @<Print string |s| as an error message@>;
flush_string;
@z

@x [51] Open detached tracing after initialization.
start_of_TEX: @<Initialize the output routines@>;
@<Get the first line of input and prepare to start@>;
@y
start_of_TEX: @<Initialize the output routines@>;
umber_trace_open;
@<Get the first line of input and prepare to start@>;
@z

@x [51] Close after final ordered events.
final_end: do_final_end;
end {|main_body|};
@y
final_end: umber_trace_finish; do_final_end;
end {|main_body|};
@z

@x [53] Observe a committed expanded write.
mubyte_out := mubyte_sout; mubyte_log := mubyte_slog;
@y
mubyte_out := mubyte_sout; mubyte_log := mubyte_slog;
umber_trace_effect(1,j,1,def_ref);
@z

@x [53] Observe committed output stream open and close effects.
  else  begin if write_open[j] then begin a_close(write_file[j]);
                                          write_open[j]:=false; end;
    if subtype(p)=close_node then do_nothing {already closed}
    else if j<16 then
@y
  else  begin if write_open[j] then begin a_close(write_file[j]);
      write_open[j]:=false;
      {Section 1374 closes the stream without naming it: only the open branch
      below assigns cur_name, cur_area, and cur_ext, and TeX retains no name
      for an open stream (section 1378 closes the survivors the same way).
      A file-name value here would publish whatever those globals last held,
      such as the most recent input file or the job name that section 529's
      pack_job_name leaves behind. So the close effect carries no value, and
      the stream number in the channel is its whole committed identity.}
      umber_trace_effect(3,j,4,0); end;
    if subtype(p)=close_node then do_nothing {already closed}
    else if j<16 then
@z

@x [53] Observe a committed output stream open.
      write_open[j]:=true;
      {If on first line of input, log file is not ready yet, so don't log.}
@y
      write_open[j]:=true;
      umber_trace_effect(2,j,3,0);
      {If on first line of input, log file is not ready yet, so don't log.}
@z

@x [53a] Observe current group enquiries after their semantic value commits.
current_group_level_code: cur_val:=cur_level-level_one;
current_group_type_code: cur_val:=cur_group;
@y
current_group_level_code: begin cur_val:=cur_level-level_one;
  umber_trace_scanner(8,int_val);
  end;
current_group_type_code: begin cur_val:=cur_group;
  umber_trace_scanner(9,int_val);
  end;
@z

@x [53a] Observe the current conditional depth enquiry.
current_if_level_code: begin q:=cond_ptr; cur_val:=0;
  while q<>null do
    begin incr(cur_val); q:=link(q);
    end;
  end;
@y
current_if_level_code: begin q:=cond_ptr; cur_val:=0;
  while q<>null do
    begin incr(cur_val); q:=link(q);
    end;
  umber_trace_scanner(10,int_val);
  end;
@z

@x [53a] Observe current conditional type and branch enquiries.
current_if_type_code: if cond_ptr=null then cur_val:=0
  else if cur_if<unless_code then cur_val:=cur_if+1
  else cur_val:=-(cur_if-unless_code+1);
current_if_branch_code:
  if (if_limit=or_code)or(if_limit=else_code) then cur_val:=1
  else if if_limit=fi_code then cur_val:=-1
  else cur_val:=0;
@y
current_if_type_code: begin
  if cond_ptr=null then cur_val:=0
  else if cur_if<unless_code then cur_val:=cur_if+1
  else cur_val:=-(cur_if-unless_code+1);
  umber_trace_scanner(11,int_val);
  end;
current_if_branch_code: begin
  if (if_limit=or_code)or(if_limit=else_code) then cur_val:=1
  else if if_limit=fi_code then cur_val:=-1
  else cur_val:=0;
  umber_trace_scanner(12,int_val);
  end;
@z

@x [53a] Observe pdfTeX unexpanded and detokenized token construction.
if odd(cur_chr) then
  begin c:=cur_chr; scan_general_text;
  if c=1 then the_toks:=cur_val
  else begin old_setting:=selector; selector:=new_string; b:=pool_ptr;
    p:=get_avail; link(p):=link(temp_head);
    token_show(p); flush_list(p);
    selector:=old_setting; the_toks:=str_toks(b);
    end;
  return;
  end
@y
if odd(cur_chr) then
  begin c:=cur_chr; scan_general_text;
  if c=1 then
    begin the_toks:=cur_val;
    umber_trace_token_list(1,7,link(temp_head),null);
    end
  else begin old_setting:=selector; selector:=new_string; b:=pool_ptr;
    p:=get_avail; link(p):=link(temp_head);
    token_show(p); flush_list(p);
    selector:=old_setting; the_toks:=str_toks(b);
    umber_trace_token_list(1,8,link(temp_head),null);
    end;
  return;
  end
@z

@x [53a] Observe the interaction-mode enquiry.
else if m=2 then cur_val:=interaction
@y
else if m=2 then
  begin cur_val:=interaction; umber_trace_scanner(19,int_val); end
@z

@x [53a] Observe the balanced token list committed to scantokens.
begin scan_general_text;
old_setting:=selector; selector:=new_string;
token_show(temp_head); selector:=old_setting;
@y
begin scan_general_text;
umber_trace_token_list(1,9,link(temp_head),null);
old_setting:=selector; selector:=new_string;
token_show(temp_head); selector:=old_setting;
@z

@x [53a] Observe protected delivery suppression at the canonical seam.
  if (cur_cmd>=call)and(cur_cmd<end_template) then
    if info(link(cur_chr))=protected_token then return;
  expand;
@y
  if (cur_cmd>=call)and(cur_cmd<end_template) then
    if info(link(cur_chr))=protected_token then
      begin umber_trace_token_splice(12,cur_tok); return; end;
  expand;
@z

@x [18] Observe e-TeX incomplete-source warning classes.
while grp_stack[in_open]<>save_ptr do
  begin decr(cur_level);
  print_nl("Warning: end of file when ");
@y
while grp_stack[in_open]<>save_ptr do
  begin decr(cur_level);
  umber_diag_report(1,"incomplete-source-nesting","group");
  print_nl("Warning: end of file when ");
@z

@x [18] Observe e-TeX incomplete conditional warning classes.
while if_stack[in_open]<>cond_ptr do
  begin print_nl("Warning: end of file when ");
@y
while if_stack[in_open]<>cond_ptr do
  begin umber_diag_report(1,"incomplete-source-nesting","conditional");
  print_nl("Warning: end of file when ");
@z

@x [53a] Observe the committed pdfTeX expression result.
arith_error:=a; cur_val:=e; cur_val_level:=l;
end;
@y
arith_error:=a; cur_val:=e; cur_val_level:=l;
umber_trace_scanner(4+l-int_val,l);
end;
@z

@x [53a] Observe glue-order enquiries after extracting their result.
  if m=glue_stretch_order_code then cur_val:=stretch_order(q)
  else cur_val:=shrink_order(q);
  delete_glue_ref(q);
  end;
@y
  if m=glue_stretch_order_code then cur_val:=stretch_order(q)
  else cur_val:=shrink_order(q);
  delete_glue_ref(q);
  if m=glue_stretch_order_code then umber_trace_scanner(13,int_val)
  else umber_trace_scanner(14,int_val);
  end;
@z

@x [53a] Observe glue-component enquiries after extracting their result.
  if m=glue_stretch_code then cur_val:=stretch(q)
  else cur_val:=shrink(q);
  delete_glue_ref(q);
  end;
@y
  if m=glue_stretch_code then cur_val:=stretch(q)
  else cur_val:=shrink(q);
  delete_glue_ref(q);
  if m=glue_stretch_code then umber_trace_scanner(15,dimen_val)
  else umber_trace_scanner(16,dimen_val);
  end;
@z

@x [53a] Observe glue/muglue conversion enquiries.
mu_to_glue_code: scan_mu_glue;
@y
mu_to_glue_code: begin scan_mu_glue;
  umber_trace_scanner(17,glue_val);
  end;
@z

@x [53a] Observe muglue/glue conversion enquiries.
glue_to_mu_code: scan_normal_glue;
@y
glue_to_mu_code: begin scan_normal_glue;
  umber_trace_scanner(18,mu_val);
  end;
@z

@x [53a] Observe committed local sparse pointer-register writes.
procedure sa_def(@!p:pointer;@!e:halfword);
  {new data for sparse array elements}
begin add_sa_ref(p);
if sa_ptr(p)=e then
  begin @!stat if tracing_assigns>0 then show_sa(p,"reassigning");@+tats@;@/
  sa_destroy(p);
  end
else  begin @!stat if tracing_assigns>0 then show_sa(p,"changing");@+tats@;@/
  if sa_lev(p)=cur_level then sa_destroy(p)@+else sa_save(p);
  sa_lev(p):=cur_level; sa_ptr(p):=e;
  @!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
  end;
delete_sa_ref(p);
end;
@#
@y
function umber_sparse_number(@!p:pointer):integer;
var n:integer;
@!q:pointer;
begin
q:=p;
if sa_index(q)<dimen_val_limit then n:=sa_num(q)
else  begin n:=hex_dig4(sa_index(q)); q:=link(q);
  n:=n+16*sa_index(q); q:=link(q);
  n:=n+256*(sa_index(q)+16*sa_index(link(q)));
  end;
umber_sparse_number:=n;
end;

procedure umber_trace_sparse_mutation(@!p:pointer;@!global_scope:boolean);
var t:small_number;
@!v:pointer;
begin
if not umber_trace_opened then return;
t:=sa_type(p);
umber_trace_begin;
write(umber_trace_file,
  '{"event":"mutation","data":{"target":"register","key":',
  '{"type":"name","value":"');
case t of
int_val:write(umber_trace_file,'count');
dimen_val:write(umber_trace_file,'dimen');
glue_val:write(umber_trace_file,'skip');
mu_val:write(umber_trace_file,'muskip');
box_val:write(umber_trace_file,'box');
othercases write(umber_trace_file,'toks')
endcases;
write(umber_trace_file,':',umber_sparse_number(p):1,'"},"value":');
if t=int_val then
  write(umber_trace_file,'{"type":"integer","value":',sa_int(p):1,'}')
else if t=dimen_val then
  write(umber_trace_file,'{"type":"scaled","value":',sa_dim(p):1,'}')
else if (t=glue_val)or(t=mu_val) then umber_trace_glue_value(sa_ptr(p))
else if t=box_val then
  begin write(umber_trace_file,'{"type":"name","value":"');
  if sa_ptr(p)=null then write(umber_trace_file,'void')
  else write(umber_trace_file,'occupied');
  write(umber_trace_file,'"}');
  end
else begin v:=sa_ptr(p);
  write(umber_trace_file,'{"type":"tokens","value":[');
  if v<>null then umber_trace_token_range(link(v),null);
  write(umber_trace_file,']}');
  end;
write(umber_trace_file,',"scope":'); umber_trace_scope(global_scope);
write_ln(umber_trace_file,'}}}');
end;

procedure sa_def(@!p:pointer;@!e:halfword);
  {new data for sparse array elements}
begin add_sa_ref(p);
if sa_ptr(p)=e then
  begin @!stat if tracing_assigns>0 then show_sa(p,"reassigning");@+tats@;@/
  sa_destroy(p);
  end
else  begin @!stat if tracing_assigns>0 then show_sa(p,"changing");@+tats@;@/
  if sa_lev(p)=cur_level then sa_destroy(p)@+else sa_save(p);
  sa_lev(p):=cur_level; sa_ptr(p):=e;
  @!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
  end;
umber_trace_sparse_mutation(p,false);
delete_sa_ref(p);
end;
@#
@z

@x [53a] Observe committed local sparse word-register writes.
procedure sa_w_def(@!p:pointer;@!w:integer);
begin add_sa_ref(p);
if sa_int(p)=w then
  begin @!stat if tracing_assigns>0 then show_sa(p,"reassigning");@+tats@;@/
  end
else  begin @!stat if tracing_assigns>0 then show_sa(p,"changing");@+tats@;@/
  if sa_lev(p)<>cur_level then sa_save(p);
  sa_lev(p):=cur_level; sa_int(p):=w;
  @!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
  end;
delete_sa_ref(p);
end;

@ The |sa_def| and |sa_w_def| routines take care of local definitions.
@y
procedure sa_w_def(@!p:pointer;@!w:integer);
begin add_sa_ref(p);
if sa_int(p)=w then
  begin @!stat if tracing_assigns>0 then show_sa(p,"reassigning");@+tats@;@/
  end
else  begin @!stat if tracing_assigns>0 then show_sa(p,"changing");@+tats@;@/
  if sa_lev(p)<>cur_level then sa_save(p);
  sa_lev(p):=cur_level; sa_int(p):=w;
  @!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
  end;
umber_trace_sparse_mutation(p,false);
delete_sa_ref(p);
end;

@ The |sa_def| and |sa_w_def| routines take care of local definitions.
@z

@x [53a] Observe committed global sparse pointer-register writes.
sa_destroy(p); sa_lev(p):=level_one; sa_ptr(p):=e;
@!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
delete_sa_ref(p);
end;
@y
sa_destroy(p); sa_lev(p):=level_one; sa_ptr(p):=e;
@!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
umber_trace_sparse_mutation(p,true);
delete_sa_ref(p);
end;
@z

@x [53a] Observe committed global sparse word-register writes.
sa_lev(p):=level_one; sa_int(p):=w;
@!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
delete_sa_ref(p);
end;
@y
sa_lev(p):=level_one; sa_int(p):=w;
@!stat if tracing_assigns>0 then show_sa(p,"into");@+tats@;@/
umber_trace_sparse_mutation(p,true);
delete_sa_ref(p);
end;
@z
