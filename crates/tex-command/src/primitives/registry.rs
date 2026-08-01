use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

/// Installs TeX82's expandable primitive meanings for a fresh INITEX state.
pub fn install_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, true);
}

/// Reconstructs TeX82's immutable primitive lookup table after format load.
pub fn register_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, false);
}

fn configure_tex82_expandable_primitives(universe: &mut Universe, install: bool) {
    for (name, primitive) in [
        ("expandafter", ExpandablePrimitive::ExpandAfter),
        ("noexpand", ExpandablePrimitive::NoExpand),
        ("csname", ExpandablePrimitive::CsName),
        ("endcsname", ExpandablePrimitive::EndCsName),
        ("string", ExpandablePrimitive::String),
        ("number", ExpandablePrimitive::Number),
        ("romannumeral", ExpandablePrimitive::RomanNumeral),
        ("meaning", ExpandablePrimitive::Meaning),
        ("the", ExpandablePrimitive::The),
        ("input", ExpandablePrimitive::Input),
        ("endinput", ExpandablePrimitive::EndInput),
        ("jobname", ExpandablePrimitive::JobName),
        ("fontname", ExpandablePrimitive::FontName),
        ("topmark", ExpandablePrimitive::TopMark),
        ("firstmark", ExpandablePrimitive::FirstMark),
        ("botmark", ExpandablePrimitive::BotMark),
        ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
        ("splitbotmark", ExpandablePrimitive::SplitBotMark),
        ("iftrue", ExpandablePrimitive::IfTrue),
        ("iffalse", ExpandablePrimitive::IfFalse),
        ("if", ExpandablePrimitive::If),
        ("ifcat", ExpandablePrimitive::IfCat),
        ("ifx", ExpandablePrimitive::IfX),
        ("ifnum", ExpandablePrimitive::IfNum),
        ("ifdim", ExpandablePrimitive::IfDim),
        ("ifodd", ExpandablePrimitive::IfOdd),
        ("ifcase", ExpandablePrimitive::IfCase),
        ("ifvmode", ExpandablePrimitive::IfVMode),
        ("ifhmode", ExpandablePrimitive::IfHMode),
        ("ifmmode", ExpandablePrimitive::IfMMode),
        ("ifinner", ExpandablePrimitive::IfInner),
        ("ifvoid", ExpandablePrimitive::IfVoid),
        ("ifhbox", ExpandablePrimitive::IfHBox),
        ("ifvbox", ExpandablePrimitive::IfVBox),
        ("ifeof", ExpandablePrimitive::IfEof),
        ("else", ExpandablePrimitive::Else),
        ("or", ExpandablePrimitive::Or),
        ("fi", ExpandablePrimitive::Fi),
    ] {
        let meaning = Meaning::ExpandablePrimitive(primitive);
        universe.register_primitive_meaning(name, meaning);
        if install {
            let symbol = universe.intern(name);
            universe.set_meaning(symbol, meaning);
        }
    }
}

/// Installs e-TeX 2.6's expandable primitive meanings for a fresh INITEX state.
pub fn install_etex_expandable_primitives(universe: &mut Universe) {
    universe.set_int_param_global(tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE, 1);
    configure_etex_expandable_primitives(universe, true);
}

/// Reconstructs e-TeX 2.6's immutable primitive lookup table after format load.
pub fn register_etex_expandable_primitives(universe: &mut Universe) {
    configure_etex_expandable_primitives(universe, false);
}

fn configure_etex_expandable_primitives(universe: &mut Universe, install: bool) {
    for (name, primitive) in [
        ("unexpanded", ExpandablePrimitive::Unexpanded),
        ("detokenize", ExpandablePrimitive::Detokenize),
        ("unless", ExpandablePrimitive::Unless),
        ("scantokens", ExpandablePrimitive::Scantokens),
        ("eTeXrevision", ExpandablePrimitive::ETeXRevision),
        ("ifdefined", ExpandablePrimitive::IfDefined),
        ("ifcsname", ExpandablePrimitive::IfCsName),
        ("iffontchar", ExpandablePrimitive::IfFontChar),
        ("topmarks", ExpandablePrimitive::TopMarks),
        ("firstmarks", ExpandablePrimitive::FirstMarks),
        ("botmarks", ExpandablePrimitive::BotMarks),
        ("splitfirstmarks", ExpandablePrimitive::SplitFirstMarks),
        ("splitbotmarks", ExpandablePrimitive::SplitBotMarks),
    ] {
        configure_primitive(
            universe,
            install,
            name,
            Meaning::ExpandablePrimitive(primitive),
        );
    }
    for (name, value) in [
        (
            "eTeXversion",
            tex_state::meaning::InternalInteger::ETeXVersion,
        ),
        (
            "currentgrouplevel",
            tex_state::meaning::InternalInteger::CurrentGroupLevel,
        ),
        (
            "currentgrouptype",
            tex_state::meaning::InternalInteger::CurrentGroupType,
        ),
        (
            "currentiflevel",
            tex_state::meaning::InternalInteger::CurrentIfLevel,
        ),
        (
            "currentiftype",
            tex_state::meaning::InternalInteger::CurrentIfType,
        ),
        (
            "currentifbranch",
            tex_state::meaning::InternalInteger::CurrentIfBranch,
        ),
        (
            "lastnodetype",
            tex_state::meaning::InternalInteger::LastNodeType,
        ),
    ] {
        configure_primitive(universe, install, name, Meaning::InternalInteger(value));
    }
}

/// Installs pdfTeX 1.40.27's implemented expandable identity surface.
pub fn install_pdftex_expandable_primitives(universe: &mut Universe) {
    configure_pdftex_expandable_primitives(universe, true);
}

/// Reconstructs pdfTeX 1.40.27's expandable primitive lookup table after a format load.
pub fn register_pdftex_expandable_primitives(universe: &mut Universe) {
    configure_pdftex_expandable_primitives(universe, false);
}

fn configure_pdftex_expandable_primitives(universe: &mut Universe, install: bool) {
    for (name, primitive) in [
        ("expanded", ExpandablePrimitive::Expanded),
        ("ifincsname", ExpandablePrimitive::IfInCsName),
        ("pdftexrevision", ExpandablePrimitive::PdfTeXRevision),
        ("pdftexbanner", ExpandablePrimitive::PdfTeXBanner),
        ("pdffontsize", ExpandablePrimitive::PdfFontSize),
        ("pdffontname", ExpandablePrimitive::PdfFontName),
        ("pdffontobjnum", ExpandablePrimitive::PdfFontObjectNumber),
        ("leftmarginkern", ExpandablePrimitive::LeftMarginKern),
        ("rightmarginkern", ExpandablePrimitive::RightMarginKern),
        ("pdfprimitive", ExpandablePrimitive::PdfPrimitive),
        ("ifpdfprimitive", ExpandablePrimitive::IfPdfPrimitive),
        ("ifpdfabsnum", ExpandablePrimitive::IfPdfAbsNum),
        ("ifpdfabsdim", ExpandablePrimitive::IfPdfAbsDim),
        ("pdfescapestring", ExpandablePrimitive::PdfEscapeString),
        ("pdfescapename", ExpandablePrimitive::PdfEscapeName),
        ("pdfescapehex", ExpandablePrimitive::PdfEscapeHex),
        ("pdfunescapehex", ExpandablePrimitive::PdfUnescapeHex),
        ("pdfstrcmp", ExpandablePrimitive::StringCompare),
        ("pdfcreationdate", ExpandablePrimitive::CreationDate),
        (
            "pdffilemoddate",
            ExpandablePrimitive::PdfFileModificationDate,
        ),
        ("pdffilesize", ExpandablePrimitive::FileSize),
        ("pdfmdfivesum", ExpandablePrimitive::PdfMdFiveSum),
        ("pdffiledump", ExpandablePrimitive::PdfFileDump),
        ("pdfmatch", ExpandablePrimitive::PdfMatch),
        ("pdflastmatch", ExpandablePrimitive::PdfLastMatch),
        ("pdfuniformdeviate", ExpandablePrimitive::PdfUniformDeviate),
        ("pdfnormaldeviate", ExpandablePrimitive::PdfNormalDeviate),
        ("pdfinsertht", ExpandablePrimitive::PdfInsertHeight),
        ("pdfximagebbox", ExpandablePrimitive::PdfXImageBBox),
        ("pdfcolorstackinit", ExpandablePrimitive::PdfColorStackInit),
        ("pdfxformname", ExpandablePrimitive::PdfXFormName),
        ("pdfpageref", ExpandablePrimitive::PdfPageRef),
    ] {
        configure_primitive(
            universe,
            install,
            name,
            Meaning::ExpandablePrimitive(primitive),
        );
    }
    for (name, value) in [
        (
            "pdftexversion",
            tex_state::meaning::InternalInteger::PdfTeXVersion,
        ),
        (
            "pdflastobj",
            tex_state::meaning::InternalInteger::PdfLastObject,
        ),
        (
            "pdflastxform",
            tex_state::meaning::InternalInteger::PdfLastXForm,
        ),
    ] {
        configure_primitive(universe, install, name, Meaning::InternalInteger(value));
    }
}

fn configure_primitive(universe: &mut Universe, install: bool, name: &str, meaning: Meaning) {
    universe.register_primitive_meaning(name, meaning);
    if install {
        let symbol = universe.intern(name);
        universe.set_meaning(symbol, meaning);
    }
}

#[cfg(test)]
mod tests {
    use tex_state::token::{OriginId, Token, TracedTokenWord};

    use super::*;
    use crate::command::{CurrentCommand, DeliveryStamp};
    use crate::processor::{PrintCommand, print_cmd_chr_text};

    fn tex82_input_mark_and_conditional_primitives() -> [(&'static str, ExpandablePrimitive); 24] {
        [
            ("input", ExpandablePrimitive::Input),
            ("endinput", ExpandablePrimitive::EndInput),
            ("topmark", ExpandablePrimitive::TopMark),
            ("firstmark", ExpandablePrimitive::FirstMark),
            ("botmark", ExpandablePrimitive::BotMark),
            ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
            ("splitbotmark", ExpandablePrimitive::SplitBotMark),
            ("if", ExpandablePrimitive::If),
            ("ifcat", ExpandablePrimitive::IfCat),
            ("ifx", ExpandablePrimitive::IfX),
            ("ifnum", ExpandablePrimitive::IfNum),
            ("ifdim", ExpandablePrimitive::IfDim),
            ("ifodd", ExpandablePrimitive::IfOdd),
            ("ifcase", ExpandablePrimitive::IfCase),
            ("ifvmode", ExpandablePrimitive::IfVMode),
            ("ifhmode", ExpandablePrimitive::IfHMode),
            ("ifmmode", ExpandablePrimitive::IfMMode),
            ("ifinner", ExpandablePrimitive::IfInner),
            ("ifvoid", ExpandablePrimitive::IfVoid),
            ("ifhbox", ExpandablePrimitive::IfHBox),
            ("ifvbox", ExpandablePrimitive::IfVBox),
            ("ifeof", ExpandablePrimitive::IfEof),
            ("iftrue", ExpandablePrimitive::IfTrue),
            ("iffalse", ExpandablePrimitive::IfFalse),
        ]
    }

    fn print_token(universe: &mut Universe, token: Token) -> String {
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                TracedTokenWord::pack(token, OriginId::UNKNOWN),
                DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };
        print_cmd_chr_text(
            &universe.command_context(),
            PrintCommand::from_current(&command),
        )
    }

    #[test]
    fn all_tex82_mark_and_conditional_primitives_survive_fresh_and_loaded_registration() {
        let cases = tex82_input_mark_and_conditional_primitives();

        let mut fresh = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut fresh);
        for (name, primitive) in cases {
            let meaning = Meaning::ExpandablePrimitive(primitive);
            let symbol = fresh.symbol(name).expect("installed control sequence");
            assert_eq!(fresh.meaning(symbol), meaning, "live meaning for \\{name}");
            assert_eq!(
                fresh.primitive_meaning(name),
                Some(meaning),
                "immutable meaning for \\{name}"
            );
            assert_eq!(
                print_token(&mut fresh, Token::Cs(symbol.symbol())),
                format!("\\{name}"),
                "print_cmd_chr spelling for installed \\{name}"
            );
        }

        let replacement = Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand);
        let mut loaded = Universe::new_with_plain_catcodes();
        for (name, _) in cases {
            let symbol = loaded.intern(name);
            loaded.set_meaning(symbol, replacement);
        }
        register_tex82_expandable_primitives(&mut loaded);

        for (name, primitive) in cases {
            let meaning = Meaning::ExpandablePrimitive(primitive);
            let symbol = loaded.symbol(name).expect("prepopulated control sequence");
            assert_eq!(
                loaded.meaning(symbol),
                replacement,
                "format meaning for \\{name} must survive registry reconstruction"
            );
            assert_eq!(
                loaded.primitive_meaning(name),
                Some(meaning),
                "reconstructed immutable meaning for \\{name}"
            );
            let frozen = loaded
                .primitive_token(name)
                .expect("frozen primitive token");
            assert_eq!(
                print_token(&mut loaded, frozen),
                format!("\\{name}"),
                "print_cmd_chr spelling for reconstructed \\{name}"
            );
        }
    }

    #[test]
    fn install_and_register_preserve_format_meanings() {
        let mut installed = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut installed);
        let iftrue = installed.intern("iftrue");
        assert_eq!(
            installed.meaning(iftrue),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue)
        );

        let mut registered = Universe::new_with_plain_catcodes();
        let replacement = Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand);
        let iftrue = registered.intern("iftrue");
        registered.set_meaning(iftrue, replacement);
        register_tex82_expandable_primitives(&mut registered);
        assert_eq!(registered.meaning(iftrue), replacement);
        assert_eq!(
            registered.primitive_meaning("iftrue"),
            Some(Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue))
        );
    }

    #[test]
    fn extension_registries_are_profile_gated_and_preserve_format_meanings() {
        let mut tex82 = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut tex82);
        assert_eq!(tex82.primitive_meaning("ifdefined"), None);
        assert_eq!(tex82.primitive_meaning("pdfprimitive"), None);

        install_etex_expandable_primitives(&mut tex82);
        let ifdefined = tex82.intern("ifdefined");
        assert_eq!(
            tex82.meaning(ifdefined),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDefined)
        );
        assert_eq!(tex82.primitive_meaning("pdfprimitive"), None);

        install_pdftex_expandable_primitives(&mut tex82);
        let pdfprimitive = tex82.intern("pdfprimitive");
        assert_eq!(
            tex82.meaning(pdfprimitive),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfPrimitive)
        );

        let replacement = Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand);
        let symbol = tex82.intern("ifdefined");
        tex82.set_meaning(symbol, replacement);
        register_etex_expandable_primitives(&mut tex82);
        assert_eq!(tex82.meaning(symbol), replacement);
        assert_eq!(
            tex82.primitive_meaning("ifdefined"),
            Some(Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDefined))
        );
        assert_eq!(
            tex82.primitive_meaning("detokenize"),
            Some(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::Detokenize
            )),
            "format loading must rebuild the immutable primitive lookup"
        );

        let symbol = tex82.intern("pdfprimitive");
        tex82.set_meaning(symbol, replacement);
        register_pdftex_expandable_primitives(&mut tex82);
        assert_eq!(tex82.meaning(symbol), replacement);
        assert_eq!(
            tex82.primitive_meaning("pdfprimitive"),
            Some(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::PdfPrimitive
            ))
        );
    }
}
