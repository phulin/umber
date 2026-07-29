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
