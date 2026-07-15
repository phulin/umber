use super::*;

pub(super) fn read_int_variable(stores: &Universe, target: Variable) -> i32 {
    match target {
        Variable::IntRegister(index) => stores.count(index),
        Variable::IntParam(index) => stores.int_param(IntParam::new(index)),
        Variable::PageInteger(integer) => stores.page_integer(integer),
        Variable::FontHyphenChar(font) => stores.font_hyphen_char(font),
        Variable::FontSkewChar(font) => stores.font_skew_char(font),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn write_int_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: i32,
    global: bool,
) {
    match target {
        Variable::IntRegister(_) => set_int_register(stores, index, value, global),
        Variable::IntParam(_) => set_int_param(stores, index, value, global),
        Variable::PageInteger(integer) => stores.set_page_integer(integer, value),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn read_dimen_variable(stores: &Universe, target: Variable) -> Scaled {
    match target {
        Variable::DimenRegister(index) => stores.dimen(index),
        Variable::DimenParam(index) => stores.dimen_param(DimenParam::new(index)),
        Variable::PageDimension(dimension) => stores.page_dimension(dimension),
        Variable::FontDimen(font, number) => stores.font_dimen(font, number),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn write_dimen_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: Scaled,
    global: bool,
) {
    match target {
        Variable::DimenRegister(_) => set_dimen_register(stores, index, value, global),
        Variable::DimenParam(_) => set_dimen_param(stores, index, value, global),
        Variable::PageDimension(dimension) => stores.set_page_dimension(dimension, value),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn read_glue_variable(stores: &Universe, target: Variable) -> GlueId {
    match target {
        Variable::GlueRegister(index) => stores.skip(index),
        Variable::GlueParam(index) => stores.glue_param(GlueParam::new(index)),
        Variable::MuGlueParam(index) => stores.glue_param(GlueParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn write_glue_variable(
    stores: &mut Universe,
    target: Variable,
    index: u16,
    value: GlueId,
    global: bool,
) {
    match target {
        Variable::GlueRegister(_) => set_glue_register(stores, index, value, global),
        Variable::GlueParam(_) => set_glue_param(stores, index, value, global),
        Variable::MuGlueParam(_) => set_glue_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn write_font_int_variable(
    stores: &mut Universe,
    target: Variable,
    font: FontId,
    value: i32,
) {
    match target {
        Variable::FontHyphenChar(_) => stores.set_font_hyphen_char(font, value),
        Variable::FontSkewChar(_) => stores.set_font_skew_char(font, value),
        _ => unreachable!("caller restricts target"),
    }
}

pub(super) fn set_int_register(stores: &mut Universe, index: u16, value: i32, global: bool) {
    if global {
        stores.set_count_global(index, value);
    } else {
        stores.set_count(index, value);
    }
}

pub(super) fn set_dimen_register(stores: &mut Universe, index: u16, value: Scaled, global: bool) {
    if global {
        stores.set_dimen_global(index, value);
    } else {
        stores.set_dimen(index, value);
    }
}

pub(super) fn set_glue_register(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_skip_global(index, value);
    } else {
        stores.set_skip(index, value);
    }
}

pub(super) fn set_muglue_register(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_muskip_global(index, value);
    } else {
        stores.set_muskip(index, value);
    }
}

pub(super) fn set_toks_register(
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

pub(super) fn set_int_param(stores: &mut Universe, index: u16, value: i32, global: bool) {
    let param = IntParam::new(index);
    if global {
        stores.set_int_param_global(param, value);
    } else {
        stores.set_int_param(param, value);
    }
}

pub(super) fn set_dimen_param(stores: &mut Universe, index: u16, value: Scaled, global: bool) {
    let param = DimenParam::new(index);
    if global {
        stores.set_dimen_param_global(param, value);
    } else {
        stores.set_dimen_param(param, value);
    }
}

pub(super) fn set_glue_param(stores: &mut Universe, index: u16, value: GlueId, global: bool) {
    let param = GlueParam::new(index);
    if global {
        stores.set_glue_param_global(param, value);
    } else {
        stores.set_glue_param(param, value);
    }
}

pub(super) fn set_tok_param(
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
