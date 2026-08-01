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
    use super::*;
    use tex_state::meaning::InternalInteger;

    type PrimitiveCase = (&'static str, Meaning);

    macro_rules! expandable_cases {
        ($(($name:literal, $variant:ident)),+ $(,)?) => {
            &[$(($name, Meaning::ExpandablePrimitive(ExpandablePrimitive::$variant))),+]
        };
    }

    const TEX82: &[PrimitiveCase] = expandable_cases![
        ("expandafter", ExpandAfter),
        ("noexpand", NoExpand),
        ("csname", CsName),
        ("endcsname", EndCsName),
        ("string", String),
        ("number", Number),
        ("romannumeral", RomanNumeral),
        ("meaning", Meaning),
        ("the", The),
        ("input", Input),
        ("endinput", EndInput),
        ("jobname", JobName),
        ("fontname", FontName),
        ("topmark", TopMark),
        ("firstmark", FirstMark),
        ("botmark", BotMark),
        ("splitfirstmark", SplitFirstMark),
        ("splitbotmark", SplitBotMark),
        ("iftrue", IfTrue),
        ("iffalse", IfFalse),
        ("if", If),
        ("ifcat", IfCat),
        ("ifx", IfX),
        ("ifnum", IfNum),
        ("ifdim", IfDim),
        ("ifodd", IfOdd),
        ("ifcase", IfCase),
        ("ifvmode", IfVMode),
        ("ifhmode", IfHMode),
        ("ifmmode", IfMMode),
        ("ifinner", IfInner),
        ("ifvoid", IfVoid),
        ("ifhbox", IfHBox),
        ("ifvbox", IfVBox),
        ("ifeof", IfEof),
        ("else", Else),
        ("or", Or),
        ("fi", Fi),
    ];

    const ETEX_EXPANDABLE: &[PrimitiveCase] = expandable_cases![
        ("unexpanded", Unexpanded),
        ("detokenize", Detokenize),
        ("unless", Unless),
        ("scantokens", Scantokens),
        ("eTeXrevision", ETeXRevision),
        ("ifdefined", IfDefined),
        ("ifcsname", IfCsName),
        ("iffontchar", IfFontChar),
        ("topmarks", TopMarks),
        ("firstmarks", FirstMarks),
        ("botmarks", BotMarks),
        ("splitfirstmarks", SplitFirstMarks),
        ("splitbotmarks", SplitBotMarks),
    ];

    const ETEX_INTEGERS: &[PrimitiveCase] = &[
        (
            "eTeXversion",
            Meaning::InternalInteger(InternalInteger::ETeXVersion),
        ),
        (
            "currentgrouplevel",
            Meaning::InternalInteger(InternalInteger::CurrentGroupLevel),
        ),
        (
            "currentgrouptype",
            Meaning::InternalInteger(InternalInteger::CurrentGroupType),
        ),
        (
            "currentiflevel",
            Meaning::InternalInteger(InternalInteger::CurrentIfLevel),
        ),
        (
            "currentiftype",
            Meaning::InternalInteger(InternalInteger::CurrentIfType),
        ),
        (
            "currentifbranch",
            Meaning::InternalInteger(InternalInteger::CurrentIfBranch),
        ),
        (
            "lastnodetype",
            Meaning::InternalInteger(InternalInteger::LastNodeType),
        ),
    ];

    const PDFTEX_EXPANDABLE: &[PrimitiveCase] = expandable_cases![
        ("expanded", Expanded),
        ("ifincsname", IfInCsName),
        ("pdftexrevision", PdfTeXRevision),
        ("pdftexbanner", PdfTeXBanner),
        ("pdffontsize", PdfFontSize),
        ("pdffontname", PdfFontName),
        ("pdffontobjnum", PdfFontObjectNumber),
        ("leftmarginkern", LeftMarginKern),
        ("rightmarginkern", RightMarginKern),
        ("pdfprimitive", PdfPrimitive),
        ("ifpdfprimitive", IfPdfPrimitive),
        ("ifpdfabsnum", IfPdfAbsNum),
        ("ifpdfabsdim", IfPdfAbsDim),
        ("pdfescapestring", PdfEscapeString),
        ("pdfescapename", PdfEscapeName),
        ("pdfescapehex", PdfEscapeHex),
        ("pdfunescapehex", PdfUnescapeHex),
        ("pdfstrcmp", StringCompare),
        ("pdfcreationdate", CreationDate),
        ("pdffilemoddate", PdfFileModificationDate),
        ("pdffilesize", FileSize),
        ("pdfmdfivesum", PdfMdFiveSum),
        ("pdffiledump", PdfFileDump),
        ("pdfmatch", PdfMatch),
        ("pdflastmatch", PdfLastMatch),
        ("pdfuniformdeviate", PdfUniformDeviate),
        ("pdfnormaldeviate", PdfNormalDeviate),
        ("pdfinsertht", PdfInsertHeight),
        ("pdfximagebbox", PdfXImageBBox),
        ("pdfcolorstackinit", PdfColorStackInit),
        ("pdfxformname", PdfXFormName),
        ("pdfpageref", PdfPageRef),
    ];

    const PDFTEX_INTEGERS: &[PrimitiveCase] = &[
        (
            "pdftexversion",
            Meaning::InternalInteger(InternalInteger::PdfTeXVersion),
        ),
        (
            "pdflastobj",
            Meaning::InternalInteger(InternalInteger::PdfLastObject),
        ),
        (
            "pdflastxform",
            Meaning::InternalInteger(InternalInteger::PdfLastXForm),
        ),
    ];

    fn etex_cases() -> impl Iterator<Item = PrimitiveCase> {
        ETEX_EXPANDABLE.iter().chain(ETEX_INTEGERS).copied()
    }

    fn pdftex_cases() -> impl Iterator<Item = PrimitiveCase> {
        PDFTEX_EXPANDABLE.iter().chain(PDFTEX_INTEGERS).copied()
    }

    fn assert_installed(universe: &Universe, cases: impl IntoIterator<Item = PrimitiveCase>) {
        let cases: Vec<_> = cases.into_iter().collect();
        for &(name, meaning) in &cases {
            let symbol = universe.symbol(name).expect("installed control sequence");
            assert_eq!(
                universe.meaning(symbol),
                meaning,
                "live meaning for \\{name}"
            );
            assert_eq!(
                universe.primitive_meaning(name),
                Some(meaning),
                "registry meaning for \\{name}"
            );
            assert_eq!(
                universe.primitive_name(meaning),
                Some(name),
                "inverse spelling for \\{name}"
            );
            let frozen = universe
                .primitive_token(name)
                .expect("frozen primitive token");
            assert_eq!(universe.frozen_primitive_name(frozen), Some(name));
            assert_eq!(universe.frozen_primitive_meaning(frozen), Some(meaning));
        }
        assert_eq!(universe.testing_primitive_count(), cases.len());
    }

    #[test]
    fn every_profile_primitive_has_a_stable_inverse_meaning() {
        let mut tex82 = Universe::new_with_plain_catcodes();
        let unrelated = tex82.intern("userprimitive");
        tex82.set_meaning(unrelated, Meaning::Relax);
        install_tex82_expandable_primitives(&mut tex82);
        assert_installed(&tex82, TEX82.iter().copied());
        assert!(etex_cases().all(|(name, _)| tex82.primitive_meaning(name).is_none()));
        assert!(pdftex_cases().all(|(name, _)| tex82.primitive_meaning(name).is_none()));
        assert_eq!(tex82.meaning(unrelated), Meaning::Relax);

        let mut etex = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut etex);
        install_etex_expandable_primitives(&mut etex);
        assert_installed(&etex, TEX82.iter().copied().chain(etex_cases()));
        assert!(pdftex_cases().all(|(name, _)| etex.primitive_meaning(name).is_none()));

        let mut pdftex = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut pdftex);
        install_etex_expandable_primitives(&mut pdftex);
        install_pdftex_expandable_primitives(&mut pdftex);
        assert_installed(
            &pdftex,
            TEX82
                .iter()
                .copied()
                .chain(etex_cases())
                .chain(pdftex_cases()),
        );
    }

    #[test]
    fn format_registration_preserves_every_shadowed_and_unrelated_meaning() {
        let all = TEX82
            .iter()
            .copied()
            .chain(etex_cases())
            .chain(pdftex_cases());
        let mut loaded = Universe::new_with_plain_catcodes();
        for (name, _) in all {
            let symbol = loaded.intern(name);
            loaded.set_meaning(symbol, Meaning::Relax);
        }
        let unrelated = loaded.intern("userprimitive");
        loaded.set_meaning(unrelated, Meaning::CharGiven('U'));

        register_tex82_expandable_primitives(&mut loaded);
        register_etex_expandable_primitives(&mut loaded);
        register_pdftex_expandable_primitives(&mut loaded);

        let all = TEX82
            .iter()
            .copied()
            .chain(etex_cases())
            .chain(pdftex_cases());
        for (name, meaning) in all {
            let symbol = loaded.symbol(name).expect("shadowed control sequence");
            assert_eq!(
                loaded.meaning(symbol),
                Meaning::Relax,
                "format shadow for \\{name}"
            );
            assert_eq!(
                loaded.primitive_meaning(name),
                Some(meaning),
                "registry meaning for \\{name}"
            );
            assert_eq!(
                loaded.primitive_name(meaning),
                Some(name),
                "inverse spelling for \\{name}"
            );
        }
        assert_eq!(loaded.meaning(unrelated), Meaning::CharGiven('U'));
        assert_eq!(loaded.primitive_meaning("userprimitive"), None);
    }
}
