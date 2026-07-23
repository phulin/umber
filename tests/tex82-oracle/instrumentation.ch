% Final transparent command-core instrumentation for canonical TeX82.
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
@!umber_recovery_insert:boolean;
@z

@x [22] Detached schema-v1 JSON Lines transport.
procedure sprint_cs(@!p:pointer); {prints a control sequence}
begin if p<hash_base then
  if p<single_base then print(p-active_base)
  else  if p<null_cs then print_esc(p-single_base)
    else  begin print_esc("csname"); print_esc("endcsname");
      end
else print_esc(text(p));
end;
@y
procedure sprint_cs(@!p:pointer); {prints a control sequence}
begin if p<hash_base then
  if p<single_base then print(p-active_base)
  else  if p<null_cs then print_esc(p-single_base)
    else  begin print_esc("csname"); print_esc("endcsname");
      end
else print_esc(text(p));
end;

procedure umber_trace_hex(@!c:integer);
var d:integer;
begin
d:=c div 16;
if d<10 then write(umber_trace_file,chr(d+48))
else write(umber_trace_file,chr(d+87));
d:=c mod 16;
if d<10 then write(umber_trace_file,chr(d+48))
else write(umber_trace_file,chr(d+87));
end;

procedure umber_trace_char(@!c:integer);
begin
if c=34 then write(umber_trace_file,'\"')
else if c=92 then write(umber_trace_file,'\\')
else if (c<32)or(c>126) then
  begin write(umber_trace_file,'\u00'); umber_trace_hex(c mod 256); end
else write(umber_trace_file,xchr[c]);
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

procedure umber_trace_command(@!expanded:boolean);
begin
if not umber_trace_opened then return;
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
  if loc>start then write(umber_trace_file,loc-start-1:1)
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
15:write(umber_trace_file,'"write"');
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
  else write(umber_trace_file,'"terminal"')
else umber_trace_input_name(t);
write_ln(umber_trace_file,'}}}');
end;

procedure umber_trace_token(@!t:halfword);
var c:integer;
begin
if t>=cs_token_flag then
  begin write(umber_trace_file,
    '{"character":0,"catcode":"escape","control_sequence":');
  umber_trace_cs(t-cs_token_flag); write(umber_trace_file,'}'); end
else begin c:=t div @'400;
  write(umber_trace_file,'{"character":',t mod @'400:1,',"catcode":');
  umber_trace_catcode(c); write(umber_trace_file,'}'); end;
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

procedure umber_trace_open;
begin umber_trace_sequence:=0; umber_recovery_insert:=false;
rewrite(umber_trace_file,'tex82-events.jsonl');
umber_trace_opened:=true;
if umber_trace_opened then write_ln(umber_trace_file,
 '{"schema":1,"manifest":',
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
else if (t=u_template)or(t=v_template) then umber_trace_input(0,3,t)
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
  umber_trace_input(1,3,token_type)
else umber_trace_input(1,1,token_type);
if token_type>=backed_up then {token list to be deleted}
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

@x [23] Observe source pushes.
name:=0; {|terminal_input| is now |true|}
end;
@y
name:=0; {|terminal_input| is now |true|}
umber_trace_input(0,0,0);
end;
@z

@x [23] Observe source retirement.
@p procedure end_file_reading;
begin first:=start; line:=line_stack[index];
if name>17 then a_close(cur_file); {forget it}
pop_input; decr(in_open);
end;
@y
@p procedure end_file_reading;
begin umber_trace_input(1,0,0);
first:=start; line:=line_stack[index];
if name>17 then a_close(cur_file); {forget it}
pop_input; decr(in_open);
end;
@z

@x [24] Observe outer validity before recovery.
begin if scanner_status<>normal then
  begin deletions_allowed:=false;
@y
begin if scanner_status<>normal then
  begin umber_trace_outer(cur_cs=0); deletions_allowed:=false;
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

@x [24] Terminal read stop is a raw zero command.
    begin cur_cmd:=0; cur_chr:=0; return;
    end;
@y
    begin cur_cmd:=0; cur_chr:=0; umber_trace_command(false);
    umber_trace_input(2,0,0); return;
    end;
@z

@x [25] noexpand scanner status.
begin save_scanner_status:=scanner_status; scanner_status:=normal;
get_token; scanner_status:=save_scanner_status; t:=cur_tok;
@y
begin save_scanner_status:=scanner_status; umber_set_scanner_status(normal);
get_token; umber_set_scanner_status(save_scanner_status); t:=cur_tok;
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

@x [25] Macro matching status.
begin scanner_status:=matching; unbalance:=0;
@y
begin umber_set_scanner_status(matching); unbalance:=0;
@z

@x [27] string/meaning scanner status.
string_code, meaning_code: begin save_scanner_status:=scanner_status;
  scanner_status:=normal; get_token; scanner_status:=save_scanner_status;
@y
string_code, meaning_code: begin save_scanner_status:=scanner_status;
  umber_set_scanner_status(normal); get_token;
  umber_set_scanner_status(save_scanner_status);
@z

@x [27] Balanced-text status entry.
begin if macro_def then scanner_status:=defining
@+else scanner_status:=absorbing;
@y
begin if macro_def then umber_set_scanner_status(defining)
@+else umber_set_scanner_status(absorbing);
@z

@x [27] Balanced-text status restoration.
found: scanner_status:=normal;
@y
found: umber_set_scanner_status(normal);
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

@x [45] Alignment status entry.
preamble:=null; cur_align:=align_head; cur_loop:=null; scanner_status:=aligning;
@y
preamble:=null; cur_align:=align_head; cur_loop:=null;
umber_set_scanner_status(aligning);
@z

@x [45] Alignment status restoration.
done: scanner_status:=normal
@y
done: umber_set_scanner_status(normal)
@z

@x [55] Open detached tracing after initialization.
start_of_TEX: @<Initialize the output routines@>;
@<Get the first line of input and prepare to start@>;
@y
start_of_TEX: @<Initialize the output routines@>;
umber_trace_open;
@<Get the first line of input and prepare to start@>;
@z

@x [55] Close after final ordered events.
final_end: do_final_end;
end {|main_body|};
@y
final_end: umber_trace_finish; do_final_end;
end {|main_body|};
@z
