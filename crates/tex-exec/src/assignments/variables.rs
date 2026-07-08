use super::*;
use tex_state::StreamSlot;
use tex_state::macro_store::MacroMeaning;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Variable {
    IntRegister(u16),
    DimenRegister(u16),
    GlueRegister(u16),
    MuGlueRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    TokParam(u16),
}

pub(super) fn execute_variable_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    let index = scan_register_index(input, stores, hooks)?;
    let target = match primitive {
        UnexpandablePrimitive::Count => Variable::IntRegister(index),
        UnexpandablePrimitive::Dimen => Variable::DimenRegister(index),
        UnexpandablePrimitive::Skip => Variable::GlueRegister(index),
        UnexpandablePrimitive::Muskip => Variable::MuGlueRegister(index),
        UnexpandablePrimitive::Toks => Variable::ToksRegister(index),
        _ => unreachable!("caller restricts primitive"),
    };
    execute_assignment_to_target(target, prefixes, input, stores, hooks)
}

pub(super) fn execute_assignment_to_target<S, H>(
    target: Variable,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) => {
            let value = scan_i32(input, stores, hooks)?;
            set_int_register(stores, index, value, global);
        }
        Variable::DimenRegister(index) => {
            let value = scan_scaled(input, stores, hooks)?;
            set_dimen_register(stores, index, value, global);
        }
        Variable::GlueRegister(index) => {
            let value = scan_glue_id(input, stores, hooks, false)?;
            set_glue_register(stores, index, value, global);
        }
        Variable::MuGlueRegister(index) => {
            let value = scan_glue_id(input, stores, hooks, true)?;
            set_muglue_register(stores, index, value, global);
        }
        Variable::ToksRegister(index) => {
            let value = scan_token_list_assignment(input, stores, hooks)?;
            set_toks_register(stores, index, value, global);
        }
        Variable::IntParam(index) => {
            let value = scan_i32(input, stores, hooks)?;
            set_int_param(stores, index, value, global);
        }
        Variable::DimenParam(index) => {
            let value = scan_scaled(input, stores, hooks)?;
            set_dimen_param(stores, index, value, global);
        }
        Variable::GlueParam(index) => {
            let value = scan_glue_id(input, stores, hooks, false)?;
            set_glue_param(stores, index, value, global);
        }
        Variable::TokParam(index) => {
            let value = scan_token_list_assignment(input, stores, hooks)?;
            set_tok_param(stores, index, value, global);
        }
    }
    Ok(())
}

pub(super) fn execute_register_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "register definition")?;
    skip_optional_equals_x(input, stores, hooks)?;
    let index = scan_register_index(input, stores, hooks)?;
    let meaning = match primitive {
        UnexpandablePrimitive::CountDef => Meaning::CountRegister(index),
        UnexpandablePrimitive::DimenDef => Meaning::DimenRegister(index),
        UnexpandablePrimitive::SkipDef => Meaning::SkipRegister(index),
        UnexpandablePrimitive::MuskipDef => Meaning::MuskipRegister(index),
        UnexpandablePrimitive::ToksDef => Meaning::ToksRegister(index),
        _ => unreachable!("caller restricts primitive"),
    };
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn execute_char_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "character definition")?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_i32(input, stores, hooks)?;
    let meaning = match primitive {
        UnexpandablePrimitive::CharDef => {
            if !(0..=255).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\chardef",
                    value,
                });
            }
            let ch = char::from_u32(value as u32).expect("0..=255 is Unicode scalar");
            Meaning::CharGiven(ch)
        }
        UnexpandablePrimitive::MathCharDef => {
            if !(0..=32_767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\mathchardef",
                    value,
                });
            }
            Meaning::MathCharGiven(value as u16)
        }
        _ => unreachable!("caller restricts primitive"),
    };
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

pub(super) fn execute_arithmetic<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_variable_target(input, stores, hooks)?;
    let _ = scan_optional_keyword_x(input, stores, hooks, "by")?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) | Variable::IntParam(index) => {
            let old = read_int_variable(stores, target);
            let rhs = scan_i32(input, stores, hooks)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            write_int_variable(stores, target, index, value, global);
        }
        Variable::DimenRegister(index) | Variable::DimenParam(index) => {
            let old = read_dimen_variable(stores, target);
            let value = match primitive {
                UnexpandablePrimitive::Advance => old
                    .checked_add(scan_scaled(input, stores, hooks)?)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                UnexpandablePrimitive::Multiply => {
                    scaled_checked_mul(old, scan_i32(input, stores, hooks)?)?
                }
                UnexpandablePrimitive::Divide => {
                    scaled_checked_div(old, scan_nonzero_i32(input, stores, hooks)?)?
                }
                _ => unreachable!("caller restricts primitive"),
            };
            write_dimen_variable(stores, target, index, value, global);
        }
        Variable::GlueRegister(index) | Variable::GlueParam(index) => {
            let old = stores.glue(read_glue_variable(stores, target));
            let rhs = scan_glue_or_factor(primitive, input, stores, hooks, false)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            write_glue_variable(stores, target, index, id, global);
        }
        Variable::MuGlueRegister(index) => {
            let old = stores.glue(stores.muskip(index));
            let rhs = scan_glue_or_factor(primitive, input, stores, hooks, true)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            set_muglue_register(stores, index, id, global);
        }
        Variable::ToksRegister(_) | Variable::TokParam(_) => {
            return Err(ExecError::UnsupportedAssignmentTarget);
        }
    }
    Ok(())
}

pub(super) fn execute_code_table_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let code = scan_i32(input, stores, hooks)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_i32(input, stores, hooks)?;
    let ch = char_from_code(code, "code-table character")?;
    match primitive {
        UnexpandablePrimitive::CatCode => stores.set_catcode(ch, catcode_from_i32(value)?),
        UnexpandablePrimitive::LcCode => {
            stores.set_lccode(ch, checked_char_code(value, "\\lccode")? as LcCode)
        }
        UnexpandablePrimitive::UcCode => {
            stores.set_uccode(ch, checked_char_code(value, "\\uccode")? as UcCode)
        }
        UnexpandablePrimitive::SfCode => {
            if !(0..=32_767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\sfcode",
                    value,
                });
            }
            stores.set_sfcode(ch, value as SfCode);
        }
        UnexpandablePrimitive::MathCode => {
            if !(0..=32_768).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\mathcode",
                    value,
                });
            }
            stores.set_mathcode(ch, value as MathCode);
        }
        UnexpandablePrimitive::DelCode => {
            if !(-1..=0xFF_FFFF).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\delcode",
                    value,
                });
            }
            stores.set_delcode(ch, value as DelCode);
        }
        _ => unreachable!("caller restricts primitive"),
    }
    Ok(())
}

pub(super) fn execute_stream_command<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let slot = scan_stream_slot(input, stores, hooks)?;
    match primitive {
        UnexpandablePrimitive::OpenIn => {
            skip_optional_equals_x(input, stores, hooks)?;
            let name = scan_file_name(input, stores, hooks, "\\openin")?;
            if stores.world_mut().open_in(slot, name).is_err() {
                stores.world_mut().close_in(slot);
            }
        }
        UnexpandablePrimitive::CloseIn => stores.world_mut().close_in(slot),
        UnexpandablePrimitive::OpenOut => {
            skip_optional_equals_x(input, stores, hooks)?;
            let name = scan_file_name(input, stores, hooks, "\\openout")?;
            stores.world_mut().open_out(slot, name);
        }
        UnexpandablePrimitive::CloseOut => stores.world_mut().close_out(slot),
        _ => unreachable!("caller restricts stream primitive"),
    }
    Ok(())
}

pub(super) fn execute_read<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let slot = scan_stream_slot(input, stores, hooks)?;
    if !scan_optional_keyword_x(input, stores, hooks, "to")? {
        return Err(ExecError::ReadNeedsTo);
    }
    let target = scan_control_sequence(input, stores, "\\read")?;
    let tokens = scan_read_tokens(slot, target, stores)?;
    let replacement_text = stores.intern_token_list(&tokens);
    let parameter_text = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        target,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter_text, replacement_text),
    );
    Ok(())
}

fn scan_read_tokens(
    slot: StreamSlot,
    target: Symbol,
    stores: &mut Universe,
) -> Result<Vec<Token>, ExecError> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut terminal_prompt = Some(read_prompt(stores, target));
    loop {
        let had_open_stream = stores
            .world()
            .stream_bufs()
            .read_stream_target(slot)
            .is_some();
        let line = if had_open_stream {
            stores.world_mut().read_stream_line(slot)?
        } else {
            read_terminal_read_line(stores, terminal_prompt.take())?
        };
        let Some(line) = line else {
            return Err(ExecError::ReadNotImplemented);
        };
        scan_read_line_tokens(&line, stores, &mut tokens, &mut depth)?;
        if depth == 0 {
            return Ok(tokens);
        }
        if had_open_stream
            && stores
                .world()
                .stream_bufs()
                .read_stream_target(slot)
                .is_none()
        {
            return Err(ExecError::FileEndedWithinRead);
        }
    }
}

fn read_terminal_read_line(
    stores: &mut Universe,
    prompt: Option<String>,
) -> Result<Option<String>, ExecError> {
    match stores.interaction_mode() {
        InteractionMode::Batch | InteractionMode::Nonstop => {
            return Err(ExecError::ReadNotImplemented);
        }
        InteractionMode::Scroll | InteractionMode::ErrorStop => {}
    }
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        prompt.as_deref().unwrap_or(""),
    );
    stores
        .world_mut()
        .read_terminal_line()?
        .map_or(Err(ExecError::TerminalReadEof), |line| Ok(Some(line)))
}

fn read_prompt(stores: &Universe, target: Symbol) -> String {
    format!("\n\\{}=", stores.resolve(target))
}

fn scan_read_line_tokens(
    line: &str,
    stores: &mut Universe,
    tokens: &mut Vec<Token>,
    depth: &mut usize,
) -> Result<(), ExecError> {
    for token in tokenize_read_line(line, stores)? {
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                *depth += 1;
                tokens.push(token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                if *depth == 0 {
                    return Ok(());
                }
                *depth -= 1;
                tokens.push(token);
            }
            _ => tokens.push(token),
        }
    }
    Ok(())
}

fn scan_stream_slot<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<StreamSlot, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks)?;
    if !(0..tex_state::world::STREAM_SLOT_COUNT as i32).contains(&value) {
        return Err(ExecError::RegisterNumberOutOfRange(value));
    }
    Ok(StreamSlot::new(value as u8))
}

fn scan_file_name<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<String, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut name = String::new();
    let Some(first) = next_non_space_x(input, stores, hooks)? else {
        return Err(ExecError::MissingToken { context });
    };
    append_file_name_token(&mut name, first, context)?;
    let mut recorder = NoopRecorder;
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
    {
        match token {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => break,
            token => append_file_name_token(&mut name, token, context)?,
        }
    }
    Ok(name)
}

fn append_file_name_token(
    name: &mut String,
    token: Token,
    context: &'static str,
) -> Result<(), ExecError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExecError::MissingToken { context }),
    }
}

fn tokenize_read_line(line: &str, stores: &mut Universe) -> Result<Vec<Token>, ExecError> {
    let mut input = InputStack::new(tex_lex::MemoryInput::new(format!("{line}\n")));
    let mut tokens = Vec::new();
    while let Some(token) = input.next_token(stores)? {
        tokens.push(token);
    }
    Ok(tokens)
}

fn read_int_variable(stores: &Universe, target: Variable) -> i32 {
    match target {
        Variable::IntRegister(index) => stores.count(index),
        Variable::IntParam(index) => stores.int_param(IntParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_int_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: i32,
    global: bool,
) {
    match target {
        Variable::IntRegister(_) => set_int_register(stores, index, value, global),
        Variable::IntParam(_) => set_int_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn read_dimen_variable(stores: &Universe, target: Variable) -> Scaled {
    match target {
        Variable::DimenRegister(index) => stores.dimen(index),
        Variable::DimenParam(index) => stores.dimen_param(DimenParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_dimen_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: Scaled,
    global: bool,
) {
    match target {
        Variable::DimenRegister(_) => set_dimen_register(stores, index, value, global),
        Variable::DimenParam(_) => set_dimen_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn read_glue_variable(stores: &Universe, target: Variable) -> GlueId {
    match target {
        Variable::GlueRegister(index) => stores.skip(index),
        Variable::GlueParam(index) => stores.glue_param(GlueParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_glue_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: GlueId,
    global: bool,
) {
    match target {
        Variable::GlueRegister(_) => set_glue_register(stores, index, value, global),
        Variable::GlueParam(_) => set_glue_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn set_int_register(stores: &mut Universe, index: u16, value: i32, global: bool) {
    if global {
        stores.set_count_global(index, value);
    } else {
        stores.set_count(index, value);
    }
}

fn set_dimen_register(stores: &mut Universe, index: u16, value: Scaled, global: bool) {
    if global {
        stores.set_dimen_global(index, value);
    } else {
        stores.set_dimen(index, value);
    }
}

fn set_glue_register(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_skip_global(index, value);
    } else {
        stores.set_skip(index, value);
    }
}

fn set_muglue_register(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_muskip_global(index, value);
    } else {
        stores.set_muskip(index, value);
    }
}

fn set_toks_register(
    stores: &mut Universe,
    index: u16,
    value: tex_state::ids::TokenListId,
    global: bool,
) {
    if global {
        stores.set_toks_global(index, value);
    } else {
        stores.set_toks(index, value);
    }
}

fn set_int_param(stores: &mut Universe, index: u16, value: i32, global: bool) {
    let param = IntParam::new(index);
    if global {
        stores.set_int_param_global(param, value);
    } else {
        stores.set_int_param(param, value);
    }
}

fn set_dimen_param(stores: &mut Universe, index: u16, value: Scaled, global: bool) {
    let param = DimenParam::new(index);
    if global {
        stores.set_dimen_param_global(param, value);
    } else {
        stores.set_dimen_param(param, value);
    }
}

fn set_glue_param(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    let param = GlueParam::new(index);
    if global {
        stores.set_glue_param_global(param, value);
    } else {
        stores.set_glue_param(param, value);
    }
}

fn set_tok_param(
    stores: &mut Universe,
    index: u16,
    value: tex_state::ids::TokenListId,
    global: bool,
) {
    let param = TokParam::new(index);
    if global {
        stores.set_tok_param_global(param, value);
    } else {
        stores.set_tok_param(param, value);
    }
}

fn char_from_code(value: i32, context: &'static str) -> Result<char, ExecError> {
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or(ExecError::InvalidCode { context, value })
}

fn checked_char_code(value: i32, context: &'static str) -> Result<u32, ExecError> {
    let _ = char_from_code(value, context)?;
    Ok(value as u32)
}

fn catcode_from_i32(value: i32) -> Result<Catcode, ExecError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(ExecError::InvalidCode {
            context: "\\catcode",
            value,
        }),
    }
}
