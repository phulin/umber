use super::*;

pub mod node {
    pub const CHAR: u8 = 0;
    pub const LIG: u8 = 1;
    pub const KERN: u8 = 2;
    pub const GLUE: u8 = 3;
    pub const PENALTY: u8 = 4;
    pub const RULE: u8 = 5;
    pub const HLIST: u8 = 6;
    pub const VLIST: u8 = 7;
    pub const WHATSIT_ANCHOR: u8 = 9;
    pub const MATH_ON: u8 = 10;
    pub const MATH_OFF: u8 = 11;
    pub const DISC: u8 = 12;
    pub const MARK: u8 = 13;
    pub const INSERT: u8 = 14;
    pub const ADJUST: u8 = 15;
    pub const MARGIN_KERN: u8 = 16;
}

pub mod leader {
    pub const NONE: u8 = 0;
    pub const HLIST: u8 = 1;
    pub const VLIST: u8 = 2;
    pub const RULE: u8 = 3;
}

pub mod effect {
    pub const OPEN_OUT: u8 = 0;
    pub const CLOSE_OUT: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const SPECIAL: u8 = 3;
    pub const PDF_ACCESSIBILITY: u8 = 4;
    pub const PDF_LITERAL: u8 = 5;
    pub const PDF_SET_MATRIX: u8 = 6;
    pub const PDF_SAVE: u8 = 7;
    pub const PDF_RESTORE: u8 = 8;
    pub const PDF_COLOR_STACK: u8 = 9;
    pub const PDF_SAVE_POSITION: u8 = 10;
    pub const PDF_SNAP_STATE: u8 = 11;
    pub const PDF_SNAP_REF_POINT: u8 = 12;
    pub const PDF_SNAP_Y: u8 = 13;
    pub const PDF_SNAP_Y_COMP: u8 = 14;
    pub const PDF_REF_XFORM: u8 = 15;
    pub const PDF_ANNOTATION: u8 = 16;
    pub const PDF_REF_XIMAGE: u8 = 17;
    pub const PDF_DESTINATION: u8 = 18;
    pub const PDF_THREAD: u8 = 19;
    pub const PDF_START_THREAD: u8 = 20;
    pub const PDF_END_THREAD: u8 = 21;
}

pub mod token {
    pub const CHAR: u8 = 0;
    pub const CONTROL_SEQUENCE: u8 = 1;
    pub const PARAM: u8 = 2;
    pub const ACTIVE_CONTROL_SEQUENCE: u8 = 3;
}

pub mod sink {
    pub const TERMINAL: u8 = 0;
    pub const LOG: u8 = 1;
    pub const TERMINAL_AND_LOG: u8 = 2;
    pub const STREAM: u8 = 3;
}

pub mod font_construction {
    pub const LOADED: u8 = 0;
    pub const COPIED: u8 = 1;
    pub const LETTERSPACED: u8 = 2;
    pub const EXPANDED: u8 = 3;
}

pub mod math_event {
    pub const START: u8 = 0;
    pub const GLYPH: u8 = 1;
    pub const RULE: u8 = 2;
    pub const END: u8 = 3;
}

pub mod math_selection {
    pub const CMAP: u8 = 0;
    pub const OUTLINE_FALLBACK: u8 = 1;
}

#[cfg(test)]
mod wire_tag_tests {
    use super::*;

    #[test]
    fn page_effect_tags_are_append_only_unique_and_bijective() {
        let tags = [
            effect::OPEN_OUT,
            effect::CLOSE_OUT,
            effect::WRITE,
            effect::SPECIAL,
            effect::PDF_ACCESSIBILITY,
            effect::PDF_LITERAL,
            effect::PDF_SET_MATRIX,
            effect::PDF_SAVE,
            effect::PDF_RESTORE,
            effect::PDF_COLOR_STACK,
            effect::PDF_SAVE_POSITION,
            effect::PDF_SNAP_STATE,
            effect::PDF_SNAP_REF_POINT,
            effect::PDF_SNAP_Y,
            effect::PDF_SNAP_Y_COMP,
            effect::PDF_REF_XFORM,
            effect::PDF_ANNOTATION,
            effect::PDF_REF_XIMAGE,
            effect::PDF_DESTINATION,
            effect::PDF_THREAD,
            effect::PDF_START_THREAD,
            effect::PDF_END_THREAD,
        ];
        for (expected, tag) in (0_u8..).zip(tags) {
            assert_eq!(tag, expected);
        }
    }
}

pub(super) fn glue_order_tag(order: GlueOrder) -> u8 {
    match order {
        GlueOrder::Normal => 0,
        GlueOrder::Fil => 1,
        GlueOrder::Fill => 2,
        GlueOrder::Filll => 3,
    }
}

pub(super) fn parse_glue_order(tag: u8) -> Result<GlueOrder, ParseError> {
    match tag {
        0 => Ok(GlueOrder::Normal),
        1 => Ok(GlueOrder::Fil),
        2 => Ok(GlueOrder::Fill),
        3 => Ok(GlueOrder::Filll),
        tag => Err(ParseError::InvalidTag {
            kind: "glue order",
            tag,
        }),
    }
}

pub(super) fn glue_sign_tag(sign: GlueSign) -> u8 {
    match sign {
        GlueSign::Normal => 0,
        GlueSign::Stretching => 1,
        GlueSign::Shrinking => 2,
    }
}

pub(super) fn parse_glue_sign(tag: u8) -> Result<GlueSign, ParseError> {
    match tag {
        0 => Ok(GlueSign::Normal),
        1 => Ok(GlueSign::Stretching),
        2 => Ok(GlueSign::Shrinking),
        tag => Err(ParseError::InvalidTag {
            kind: "glue sign",
            tag,
        }),
    }
}

pub(super) fn kern_kind_tag(kind: KernKind) -> u8 {
    match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::LeftMargin => 3,
        KernKind::RightMargin => 4,
        KernKind::Auto => 5,
    }
}

pub(super) fn parse_kern_kind(tag: u8) -> Result<KernKind, ParseError> {
    match tag {
        0 => Ok(KernKind::Explicit),
        1 => Ok(KernKind::Font),
        2 => Ok(KernKind::Accent),
        3 => Ok(KernKind::LeftMargin),
        4 => Ok(KernKind::RightMargin),
        5 => Ok(KernKind::Auto),
        tag => Err(ParseError::InvalidTag {
            kind: "kern kind",
            tag,
        }),
    }
}

pub(super) fn disc_kind_tag(kind: DiscKind) -> u8 {
    match kind {
        DiscKind::Discretionary => 0,
        DiscKind::ExplicitHyphen => 1,
        DiscKind::AutomaticHyphen => 2,
    }
}

pub(super) fn parse_disc_kind(tag: u8) -> Result<DiscKind, ParseError> {
    match tag {
        0 => Ok(DiscKind::Discretionary),
        1 => Ok(DiscKind::ExplicitHyphen),
        2 => Ok(DiscKind::AutomaticHyphen),
        tag => Err(ParseError::InvalidTag {
            kind: "disc kind",
            tag,
        }),
    }
}

pub(super) fn glue_kind_tag(kind: GlueKind) -> u8 {
    match kind {
        GlueKind::Normal => 0,
        GlueKind::BaselineSkip => 1,
        GlueKind::LineSkip => 2,
        GlueKind::LeftSkip => 3,
        GlueKind::RightSkip => 4,
        GlueKind::ParFillSkip => 5,
        GlueKind::Leaders => 6,
        GlueKind::Cleaders => 7,
        GlueKind::Xleaders => 8,
    }
}

pub(super) fn token_catcode_tag(cat: TokenCatcode) -> u8 {
    match cat {
        TokenCatcode::Escape => 0,
        TokenCatcode::BeginGroup => 1,
        TokenCatcode::EndGroup => 2,
        TokenCatcode::MathShift => 3,
        TokenCatcode::AlignmentTab => 4,
        TokenCatcode::EndLine => 5,
        TokenCatcode::Parameter => 6,
        TokenCatcode::Superscript => 7,
        TokenCatcode::Subscript => 8,
        TokenCatcode::Ignored => 9,
        TokenCatcode::Space => 10,
        TokenCatcode::Letter => 11,
        TokenCatcode::Other => 12,
        TokenCatcode::Active => 13,
        TokenCatcode::Comment => 14,
        TokenCatcode::Invalid => 15,
    }
}

pub(super) fn parse_token_catcode(tag: u8) -> Result<TokenCatcode, ParseError> {
    match tag {
        0 => Ok(TokenCatcode::Escape),
        1 => Ok(TokenCatcode::BeginGroup),
        2 => Ok(TokenCatcode::EndGroup),
        3 => Ok(TokenCatcode::MathShift),
        4 => Ok(TokenCatcode::AlignmentTab),
        5 => Ok(TokenCatcode::EndLine),
        6 => Ok(TokenCatcode::Parameter),
        7 => Ok(TokenCatcode::Superscript),
        8 => Ok(TokenCatcode::Subscript),
        9 => Ok(TokenCatcode::Ignored),
        10 => Ok(TokenCatcode::Space),
        11 => Ok(TokenCatcode::Letter),
        12 => Ok(TokenCatcode::Other),
        13 => Ok(TokenCatcode::Active),
        14 => Ok(TokenCatcode::Comment),
        15 => Ok(TokenCatcode::Invalid),
        tag => Err(ParseError::InvalidTag {
            kind: "token catcode",
            tag,
        }),
    }
}

pub(super) fn parse_glue_kind(tag: u8) -> Result<GlueKind, ParseError> {
    match tag {
        0 => Ok(GlueKind::Normal),
        1 => Ok(GlueKind::BaselineSkip),
        2 => Ok(GlueKind::LineSkip),
        3 => Ok(GlueKind::LeftSkip),
        4 => Ok(GlueKind::RightSkip),
        5 => Ok(GlueKind::ParFillSkip),
        6 => Ok(GlueKind::Leaders),
        7 => Ok(GlueKind::Cleaders),
        8 => Ok(GlueKind::Xleaders),
        tag => Err(ParseError::InvalidTag {
            kind: "glue kind",
            tag,
        }),
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn effect_tags_are_unique_and_pdf_extensions_use_the_append_only_range() {
        let tags = [
            effect::OPEN_OUT,
            effect::CLOSE_OUT,
            effect::WRITE,
            effect::SPECIAL,
            effect::PDF_ACCESSIBILITY,
            effect::PDF_ANNOTATION,
            effect::PDF_REF_XIMAGE,
            effect::PDF_DESTINATION,
        ];
        let unique = tags.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), tags.len());
        const {
            assert!(effect::PDF_ANNOTATION == 16);
            assert!(effect::PDF_DESTINATION == 18);
        };
    }
}
