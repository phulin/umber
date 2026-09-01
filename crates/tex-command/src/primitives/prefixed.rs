//! TeX82's `max_non_prefixed_command` partition of the command codes.

use tex_state::meaning::{Meaning, UnexpandablePrimitive};

/// Whether a meaning's TeX82 command code exceeds `max_non_prefixed_command`.
///
/// tex.web §209 fixes `max_non_prefixed_command=70` and gives codes 71-100 to
/// the mode-independent assignments; pdftex.web extends the range with
/// `letterspace_font=101` and `pdf_copy_font=102`. That single numeric test is
/// what §1211's `prefixed_command` dispatches, what §1270's `do_assignments`
/// loops on (`if cur_cmd<=max_non_prefixed_command then return`), and what
/// §1211's `\global` prefix validates.
///
/// It is deliberately narrower than "this command assigns something": it
/// excludes `\begingroup` (61), `\endgroup` (62), `\aftergroup` (41),
/// `\afterassignment` (40), `\openin`/`\closein` (`in_stream`, 60) and every
/// `extension` primitive (59) -- `\write`, `\special`, `\openout`,
/// `\closeout`, `\immediate`, and pdfTeX's `\pdffontattr`, `\pdfmapfile`,
/// `\pdfmapline`, `\pdffontexpand`, `\pdfincludechars`,
/// `\pdfglyphtounicode`, `\pdfnobuiltintounicode`, all of which pdftex.web
/// declares with `primitive(..., extension, ...)`. `\global` prefixes none of
/// them and `do_assignments` executes none of them. A caller that starts from
/// a broader "is an assignment" notion and subtracts those by hand is
/// re-deriving this predicate one exception at a time.
pub fn is_prefixed_command(meaning: Meaning) -> bool {
    match meaning {
        // `assign_toks` (72), `assign_int` (73), `assign_dimen` (74),
        // `assign_glue` (75), `assign_mu_glue` (76), `set_page_dimen` (81),
        // `set_page_int` (82), and the `toks_register` (71) / `register` (89)
        // slots a `\toksdef`/`\countdef` shorthand names directly.
        Meaning::TokParam(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::ToksRegister(_)
        | Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_) => true,
        // `set_font` (87): a font identifier selects the current font.
        Meaning::Font(_) => true,
        Meaning::UnexpandablePrimitive(primitive) => is_prefixed_command_primitive(primitive),
        _ => false,
    }
}

const fn is_prefixed_command_primitive(primitive: UnexpandablePrimitive) -> bool {
    matches!(
        primitive,
        // `toks_register` (71) and `register` (89).
        UnexpandablePrimitive::Toks
            | UnexpandablePrimitive::Count
            | UnexpandablePrimitive::Dimen
            | UnexpandablePrimitive::Skip
            | UnexpandablePrimitive::Muskip
            // `assign_int` (73): `\globaldefs` is an ordinary integer
            // parameter primitive, not a prefix.
            | UnexpandablePrimitive::GlobalDefs
            // `assign_font_dimen` (77) and `assign_font_int` (78), the latter
            // extended by pdftex.web's per-font code tables.
            | UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar
            | UnexpandablePrimitive::PdfLpCode
            | UnexpandablePrimitive::PdfRpCode
            | UnexpandablePrimitive::PdfEfCode
            | UnexpandablePrimitive::PdfTagCode
            | UnexpandablePrimitive::PdfKnbsCode
            | UnexpandablePrimitive::PdfStbsCode
            | UnexpandablePrimitive::PdfShbsCode
            | UnexpandablePrimitive::PdfKnbcCode
            | UnexpandablePrimitive::PdfKnacCode
            | UnexpandablePrimitive::PdfNoLigatures
            // `set_aux` (79), `set_prev_graf` (80), `set_page_int` (82).
            | UnexpandablePrimitive::SpaceFactor
            | UnexpandablePrimitive::PrevDepth
            | UnexpandablePrimitive::PrevGraf
            | UnexpandablePrimitive::InteractionMode
            // `set_box_dimen` (83).
            | UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp
            // `set_shape` (84), extended by e-TeX's penalty shapes.
            | UnexpandablePrimitive::ParShape
            | UnexpandablePrimitive::InterLinePenalties
            | UnexpandablePrimitive::ClubPenalties
            | UnexpandablePrimitive::WidowPenalties
            | UnexpandablePrimitive::DisplayWidowPenalties
            // `def_code` (85).
            | UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode
            // `def_family` (86).
            | UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont
            // `def_font` (88), `letterspace_font` (101), `pdf_copy_font`
            // (102).
            | UnexpandablePrimitive::Font
            | UnexpandablePrimitive::LetterspaceFont
            | UnexpandablePrimitive::PdfCopyFont
            // `advance` (90), `multiply` (91), `divide` (92).
            | UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide
            // `prefix` (93), extended by e-TeX's `\protected`.
            | UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected
            // `let` (94).
            | UnexpandablePrimitive::Let
            | UnexpandablePrimitive::FutureLet
            | UnexpandablePrimitive::Mubyte
            // `shorthand_def` (95).
            | UnexpandablePrimitive::CharDef
            | UnexpandablePrimitive::MathCharDef
            | UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef
            // `read_to_cs` (96), extended by e-TeX's `\readline`.
            | UnexpandablePrimitive::Read
            | UnexpandablePrimitive::ReadLine
            // `def` (97).
            | UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef
            // `set_box` (98).
            | UnexpandablePrimitive::SetBox
            // `hyph_data` (99).
            | UnexpandablePrimitive::Patterns
            | UnexpandablePrimitive::Hyphenation
            // `set_interaction` (100).
            | UnexpandablePrimitive::BatchMode
            | UnexpandablePrimitive::NonstopMode
            | UnexpandablePrimitive::ScrollMode
            | UnexpandablePrimitive::ErrorStopMode
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_commands_are_exactly_texweb_codes_above_max_non_prefixed_command() {
        // Codes 71-102: `\global` prefixes these and `do_assignments` runs
        // them.
        for primitive in [
            UnexpandablePrimitive::Toks,
            UnexpandablePrimitive::Count,
            UnexpandablePrimitive::GlobalDefs,
            UnexpandablePrimitive::ParShape,
            UnexpandablePrimitive::Wd,
            UnexpandablePrimitive::TextFont,
            UnexpandablePrimitive::Font,
            UnexpandablePrimitive::Advance,
            UnexpandablePrimitive::Global,
            UnexpandablePrimitive::Let,
            UnexpandablePrimitive::CharDef,
            UnexpandablePrimitive::Read,
            UnexpandablePrimitive::Edef,
            UnexpandablePrimitive::SetBox,
            UnexpandablePrimitive::Patterns,
            UnexpandablePrimitive::BatchMode,
            UnexpandablePrimitive::LetterspaceFont,
            UnexpandablePrimitive::PdfCopyFont,
        ] {
            assert!(
                is_prefixed_command(Meaning::UnexpandablePrimitive(primitive)),
                "{primitive:?} is a prefixed command"
            );
        }

        // Codes at or below 70, including the assignment-shaped commands
        // `\global` cannot prefix.
        for primitive in [
            UnexpandablePrimitive::BeginGroup,
            UnexpandablePrimitive::EndGroup,
            UnexpandablePrimitive::AfterGroup,
            UnexpandablePrimitive::AfterAssignment,
            UnexpandablePrimitive::OpenIn,
            UnexpandablePrimitive::CloseIn,
            UnexpandablePrimitive::OpenOut,
            UnexpandablePrimitive::CloseOut,
            UnexpandablePrimitive::Immediate,
            UnexpandablePrimitive::Write,
            UnexpandablePrimitive::Special,
            UnexpandablePrimitive::PdfFontAttr,
            UnexpandablePrimitive::PdfMapFile,
            UnexpandablePrimitive::Char,
            UnexpandablePrimitive::Accent,
            UnexpandablePrimitive::HBox,
            UnexpandablePrimitive::Par,
            UnexpandablePrimitive::LastPenalty,
        ] {
            assert!(
                !is_prefixed_command(Meaning::UnexpandablePrimitive(primitive)),
                "{primitive:?} is not a prefixed command"
            );
        }

        assert!(is_prefixed_command(Meaning::CountRegister(0)));
        assert!(is_prefixed_command(Meaning::TokParam(0)));
        assert!(is_prefixed_command(Meaning::Font(
            tex_state::font::NULL_FONT
        )));
        assert!(!is_prefixed_command(Meaning::CharGiven('a')));
        assert!(!is_prefixed_command(Meaning::MathCharGiven(0)));
        assert!(!is_prefixed_command(Meaning::Relax));
    }
}
