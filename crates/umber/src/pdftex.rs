//! Pinned pdfTeX 1.40.27 engine-layer inventory and mode registration.

use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::ids::TokenListId;
use tex_state::meaning::{InternalInteger, Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;

/// The exact 158-name layer obtained from the pinned `pdftex.web` source.
pub const PDFTEX_PRIMITIVE_NAMES: &[&str] = &[
    "efcode",
    "expanded",
    "ifincsname",
    "ifpdfabsdim",
    "ifpdfabsnum",
    "ifpdfprimitive",
    "ignoreprimitiveerror",
    "knaccode",
    "knbccode",
    "knbscode",
    "leftmarginkern",
    "letterspacefont",
    "lpcode",
    "pdfadjustinterwordglue",
    "pdfadjustspacing",
    "pdfannot",
    "pdfappendkern",
    "pdfcatalog",
    "pdfcolorstack",
    "pdfcolorstackinit",
    "pdfcompresslevel",
    "pdfcopyfont",
    "pdfcreationdate",
    "pdfdecimaldigits",
    "pdfdest",
    "pdfdestmargin",
    "pdfdraftmode",
    "pdfeachlinedepth",
    "pdfeachlineheight",
    "pdfelapsedtime",
    "pdfendlink",
    "pdfendthread",
    "pdfescapehex",
    "pdfescapename",
    "pdfescapestring",
    "pdffakespace",
    "pdffiledump",
    "pdffilemoddate",
    "pdffilesize",
    "pdffirstlineheight",
    "pdffontattr",
    "pdffontexpand",
    "pdffontname",
    "pdffontobjnum",
    "pdffontsize",
    "pdfforcepagebox",
    "pdfgamma",
    "pdfgentounicode",
    "pdfglyphtounicode",
    "pdfhorigin",
    "pdfignoreddimen",
    "pdfimageapplygamma",
    "pdfimagegamma",
    "pdfimagehicolor",
    "pdfimageresolution",
    "pdfincludechars",
    "pdfinclusioncopyfonts",
    "pdfinclusionerrorlevel",
    "pdfinfo",
    "pdfinfoomitdate",
    "pdfinsertht",
    "pdfinterwordspaceoff",
    "pdfinterwordspaceon",
    "pdflastannot",
    "pdflastlinedepth",
    "pdflastlink",
    "pdflastmatch",
    "pdflastobj",
    "pdflastxform",
    "pdflastximage",
    "pdflastximagecolordepth",
    "pdflastximagepages",
    "pdflastxpos",
    "pdflastypos",
    "pdflinkmargin",
    "pdfliteral",
    "pdfmajorversion",
    "pdfmapfile",
    "pdfmapline",
    "pdfmatch",
    "pdfmdfivesum",
    "pdfminorversion",
    "pdfmovechars",
    "pdfnames",
    "pdfnobuiltintounicode",
    "pdfnoligatures",
    "pdfnormaldeviate",
    "pdfobj",
    "pdfobjcompresslevel",
    "pdfomitcharset",
    "pdfomitinfodict",
    "pdfomitprocset",
    "pdfoptionalwaysusepdfpagebox",
    "pdfoptionpdfinclusionerrorlevel",
    "pdfoptionpdfminorversion",
    "pdfoutline",
    "pdfoutput",
    "pdfpageattr",
    "pdfpagebox",
    "pdfpageheight",
    "pdfpageref",
    "pdfpageresources",
    "pdfpagesattr",
    "pdfpagewidth",
    "pdfpkmode",
    "pdfpkresolution",
    "pdfprependkern",
    "pdfprimitive",
    "pdfprotrudechars",
    "pdfptexuseunderscore",
    "pdfpxdimen",
    "pdfrandomseed",
    "pdfrefobj",
    "pdfrefxform",
    "pdfrefximage",
    "pdfresettimer",
    "pdfrestore",
    "pdfretval",
    "pdfrunninglinkoff",
    "pdfrunninglinkon",
    "pdfsave",
    "pdfsavepos",
    "pdfsetmatrix",
    "pdfsetrandomseed",
    "pdfshellescape",
    "pdfsnaprefpoint",
    "pdfsnapy",
    "pdfsnapycomp",
    "pdfspacefont",
    "pdfstartlink",
    "pdfstartthread",
    "pdfstrcmp",
    "pdfsuppressptexinfo",
    "pdfsuppresswarningdupdest",
    "pdfsuppresswarningdupmap",
    "pdfsuppresswarningpagegroup",
    "pdftexbanner",
    "pdftexrevision",
    "pdftexversion",
    "pdfthread",
    "pdfthreadmargin",
    "pdftracingfonts",
    "pdftrailer",
    "pdftrailerid",
    "pdfunescapehex",
    "pdfuniformdeviate",
    "pdfuniqueresname",
    "pdfvorigin",
    "pdfxform",
    "pdfxformname",
    "pdfximage",
    "pdfximagebbox",
    "quitvmode",
    "rightmarginkern",
    "rpcode",
    "shbscode",
    "stbscode",
    "tagcode",
];

const PDFTEX_INT_PARAMETER_MEANINGS: &[(&str, IntParam)] = &[
    ("pdfoutput", IntParam::PDF_OUTPUT),
    ("pdfcompresslevel", IntParam::PDF_COMPRESS_LEVEL),
    ("pdfobjcompresslevel", IntParam::PDF_OBJ_COMPRESS_LEVEL),
    ("pdfdecimaldigits", IntParam::PDF_DECIMAL_DIGITS),
    ("pdfmovechars", IntParam::PDF_MOVE_CHARS),
    ("pdfimageresolution", IntParam::PDF_IMAGE_RESOLUTION),
    ("pdfpkresolution", IntParam::PDF_PK_RESOLUTION),
    ("pdfuniqueresname", IntParam::PDF_UNIQUE_RESNAME),
    ("pdfoptionpdfminorversion", IntParam::PDF_MINOR_VERSION),
    (
        "pdfoptionalwaysusepdfpagebox",
        IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX,
    ),
    (
        "pdfoptionpdfinclusionerrorlevel",
        IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL,
    ),
    ("pdfmajorversion", IntParam::PDF_MAJOR_VERSION),
    ("pdfminorversion", IntParam::PDF_MINOR_VERSION),
    ("pdfforcepagebox", IntParam::PDF_FORCE_PAGE_BOX),
    ("pdfpagebox", IntParam::PDF_PAGE_BOX),
    (
        "pdfinclusionerrorlevel",
        IntParam::PDF_INCLUSION_ERROR_LEVEL,
    ),
    ("pdfgamma", IntParam::PDF_GAMMA),
    ("pdfimagegamma", IntParam::PDF_IMAGE_GAMMA),
    ("pdfimagehicolor", IntParam::PDF_IMAGE_HICOLOR),
    ("pdfimageapplygamma", IntParam::PDF_IMAGE_APPLY_GAMMA),
    ("pdfadjustspacing", IntParam::PDF_ADJUST_SPACING),
    ("pdfprotrudechars", IntParam::PDF_PROTRUDE_CHARS),
    ("pdftracingfonts", IntParam::PDF_TRACING_FONTS),
    (
        "pdfadjustinterwordglue",
        IntParam::PDF_ADJUST_INTERWORD_GLUE,
    ),
    ("pdfprependkern", IntParam::PDF_PREPEND_KERN),
    ("pdfappendkern", IntParam::PDF_APPEND_KERN),
    ("pdfgentounicode", IntParam::PDF_GEN_TO_UNICODE),
    ("pdfdraftmode", IntParam::PDF_DRAFT_MODE),
    ("pdfinclusioncopyfonts", IntParam::PDF_INCLUSION_COPY_FONTS),
    (
        "pdfsuppresswarningdupdest",
        IntParam::PDF_SUPPRESS_WARNING_DUP_DEST,
    ),
    (
        "pdfsuppresswarningdupmap",
        IntParam::PDF_SUPPRESS_WARNING_DUP_MAP,
    ),
    (
        "pdfsuppresswarningpagegroup",
        IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP,
    ),
    ("pdfinfoomitdate", IntParam::PDF_INFO_OMIT_DATE),
    ("pdfsuppressptexinfo", IntParam::PDF_SUPPRESS_PTEX_INFO),
    ("pdfomitcharset", IntParam::PDF_OMIT_CHARSET),
    ("pdfomitinfodict", IntParam::PDF_OMIT_INFO_DICT),
    ("pdfomitprocset", IntParam::PDF_OMIT_PROCSET),
    ("pdfptexuseunderscore", IntParam::PDF_PTEX_USE_UNDERSCORE),
];

const PDFTEX_INT_PARAMETER_DEFAULTS: &[(IntParam, i32)] = &[
    (IntParam::PDF_OUTPUT, 0),
    (IntParam::PDF_COMPRESS_LEVEL, 9),
    (IntParam::PDF_OBJ_COMPRESS_LEVEL, 0),
    (IntParam::PDF_DECIMAL_DIGITS, 3),
    (IntParam::PDF_MOVE_CHARS, 0),
    (IntParam::PDF_IMAGE_RESOLUTION, 72),
    (IntParam::PDF_PK_RESOLUTION, 0),
    (IntParam::PDF_UNIQUE_RESNAME, 0),
    (IntParam::PDF_MINOR_VERSION, 4),
    (IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX, 0),
    (IntParam::PDF_FORCE_PAGE_BOX, 0),
    (IntParam::PDF_PAGE_BOX, 0),
    (IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL, 0),
    (IntParam::PDF_INCLUSION_ERROR_LEVEL, 0),
    (IntParam::PDF_MAJOR_VERSION, 1),
    (IntParam::PDF_GAMMA, 1000),
    (IntParam::PDF_IMAGE_GAMMA, 2200),
    (IntParam::PDF_IMAGE_HICOLOR, 1),
    (IntParam::PDF_IMAGE_APPLY_GAMMA, 0),
    (IntParam::PDF_ADJUST_SPACING, 0),
    (IntParam::PDF_PROTRUDE_CHARS, 0),
    (IntParam::PDF_TRACING_FONTS, 0),
    (IntParam::PDF_ADJUST_INTERWORD_GLUE, 0),
    (IntParam::PDF_PREPEND_KERN, 0),
    (IntParam::PDF_APPEND_KERN, 0),
    (IntParam::PDF_GEN_TO_UNICODE, 0),
    (IntParam::PDF_DRAFT_MODE, 0),
    (IntParam::PDF_INCLUSION_COPY_FONTS, 0),
    (IntParam::PDF_SUPPRESS_WARNING_DUP_DEST, 0),
    (IntParam::PDF_SUPPRESS_WARNING_DUP_MAP, 0),
    (IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP, 0),
    (IntParam::PDF_INFO_OMIT_DATE, 0),
    (IntParam::PDF_SUPPRESS_PTEX_INFO, 0),
    (IntParam::PDF_OMIT_CHARSET, 0),
    (IntParam::PDF_OMIT_INFO_DICT, 0),
    (IntParam::PDF_OMIT_PROCSET, 0),
    (IntParam::PDF_PTEX_USE_UNDERSCORE, 0),
];

const PDFTEX_DIMEN_PARAMETERS: &[(&str, DimenParam, i32)] = &[
    ("pdfhorigin", DimenParam::PDF_H_ORIGIN, 4_736_287),
    ("pdfvorigin", DimenParam::PDF_V_ORIGIN, 4_736_287),
    ("pdfpagewidth", DimenParam::PDF_PAGE_WIDTH, 0),
    ("pdfpageheight", DimenParam::PDF_PAGE_HEIGHT, 0),
    ("pdflinkmargin", DimenParam::PDF_LINK_MARGIN, 0),
    ("pdfdestmargin", DimenParam::PDF_DEST_MARGIN, 0),
    ("pdfthreadmargin", DimenParam::PDF_THREAD_MARGIN, 0),
    (
        "pdffirstlineheight",
        DimenParam::PDF_FIRST_LINE_HEIGHT,
        -65_536_000,
    ),
    (
        "pdflastlinedepth",
        DimenParam::PDF_LAST_LINE_DEPTH,
        -65_536_000,
    ),
    (
        "pdfeachlineheight",
        DimenParam::PDF_EACH_LINE_HEIGHT,
        -65_536_000,
    ),
    (
        "pdfeachlinedepth",
        DimenParam::PDF_EACH_LINE_DEPTH,
        -65_536_000,
    ),
    (
        "pdfignoreddimen",
        DimenParam::PDF_IGNORED_DIMEN,
        -65_536_000,
    ),
    ("pdfpxdimen", DimenParam::PDF_PX_DIMEN, 65_782),
];

const PDFTEX_TOK_PARAMETERS: &[(&str, TokParam)] = &[
    ("pdfpagesattr", TokParam::PDF_PAGES_ATTR),
    ("pdfpageattr", TokParam::PDF_PAGE_ATTR),
    ("pdfpageresources", TokParam::PDF_PAGE_RESOURCES),
    ("pdfpkmode", TokParam::PDF_PK_MODE),
];

pub(crate) fn install_pdftex_layer(stores: &mut Universe) {
    for &name in PDFTEX_PRIMITIVE_NAMES {
        if name == "ifincsname" {
            // pdfTeX inherits this exact primitive from its e-TeX layer.
            continue;
        }
        let symbol = stores.intern(name);
        stores.set_meaning(
            symbol,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfTeXUnimplemented),
        );
    }
    for &(name, parameter) in PDFTEX_INT_PARAMETER_MEANINGS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::IntParam(parameter.raw()));
    }
    for &(name, parameter, _) in PDFTEX_DIMEN_PARAMETERS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::DimenParam(parameter.raw()));
    }
    for &(name, parameter) in PDFTEX_TOK_PARAMETERS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::TokParam(parameter.raw()));
    }
    for &(name, primitive) in &[
        ("lpcode", UnexpandablePrimitive::PdfLpCode),
        ("rpcode", UnexpandablePrimitive::PdfRpCode),
        ("efcode", UnexpandablePrimitive::PdfEfCode),
        ("tagcode", UnexpandablePrimitive::PdfTagCode),
        ("knbscode", UnexpandablePrimitive::PdfKnbsCode),
        ("stbscode", UnexpandablePrimitive::PdfStbsCode),
        ("shbscode", UnexpandablePrimitive::PdfShbsCode),
        ("knbccode", UnexpandablePrimitive::PdfKnbcCode),
        ("knaccode", UnexpandablePrimitive::PdfKnacCode),
        ("pdfnoligatures", UnexpandablePrimitive::PdfNoLigatures),
        ("letterspacefont", UnexpandablePrimitive::LetterspaceFont),
        ("pdfcopyfont", UnexpandablePrimitive::PdfCopyFont),
        ("pdffontexpand", UnexpandablePrimitive::PdfFontExpand),
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
        ("pdfincludechars", UnexpandablePrimitive::PdfIncludeChars),
        ("pdfmapfile", UnexpandablePrimitive::PdfMapFile),
        ("pdfmapline", UnexpandablePrimitive::PdfMapLine),
        (
            "pdfglyphtounicode",
            UnexpandablePrimitive::PdfGlyphToUnicode,
        ),
        (
            "pdfnobuiltintounicode",
            UnexpandablePrimitive::PdfNoBuiltinToUnicode,
        ),
        ("pdfliteral", UnexpandablePrimitive::PdfLiteral),
        ("pdfsetmatrix", UnexpandablePrimitive::PdfSetMatrix),
        ("pdfsave", UnexpandablePrimitive::PdfSave),
        ("pdfrestore", UnexpandablePrimitive::PdfRestore),
        ("pdfcolorstack", UnexpandablePrimitive::PdfColorStack),
        ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
        ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
        ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
        ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
        ("pdfxform", UnexpandablePrimitive::PdfXForm),
        ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    for (name, integer) in [
        ("pdfelapsedtime", InternalInteger::PdfElapsedTime),
        ("pdfrandomseed", InternalInteger::PdfRandomSeed),
        ("pdfshellescape", InternalInteger::PdfShellEscape),
        ("pdflastannot", InternalInteger::PdfLastAnnot),
        ("pdflastlink", InternalInteger::PdfLastLink),
        ("pdflastxpos", InternalInteger::PdfLastXPos),
        ("pdflastypos", InternalInteger::PdfLastYPos),
        ("pdflastxform", InternalInteger::PdfLastXForm),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::InternalInteger(integer));
    }
    for (name, primitive) in [
        ("pdfresettimer", UnexpandablePrimitive::PdfResetTimer),
        ("pdfsetrandomseed", UnexpandablePrimitive::PdfSetRandomSeed),
        ("pdfobj", UnexpandablePrimitive::PdfObject),
        ("pdfrefobj", UnexpandablePrimitive::PdfReferenceObject),
        ("pdfinfo", UnexpandablePrimitive::PdfInfo),
        ("pdfcatalog", UnexpandablePrimitive::PdfCatalog),
        ("pdfnames", UnexpandablePrimitive::PdfNames),
        ("pdftrailer", UnexpandablePrimitive::PdfTrailer),
        ("pdftrailerid", UnexpandablePrimitive::PdfTrailerId),
        (
            "pdfinterwordspaceon",
            UnexpandablePrimitive::PdfInterwordSpaceOn,
        ),
        (
            "pdfinterwordspaceoff",
            UnexpandablePrimitive::PdfInterwordSpaceOff,
        ),
        ("pdffakespace", UnexpandablePrimitive::PdfFakeSpace),
        ("pdfspacefont", UnexpandablePrimitive::PdfSpaceFont),
        ("pdfannot", UnexpandablePrimitive::PdfAnnot),
        ("pdfstartlink", UnexpandablePrimitive::PdfStartLink),
        ("pdfendlink", UnexpandablePrimitive::PdfEndLink),
        ("pdfrunninglinkon", UnexpandablePrimitive::PdfRunningLinkOn),
        (
            "pdfrunninglinkoff",
            UnexpandablePrimitive::PdfRunningLinkOff,
        ),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    tex_expand::install_pdftex_expandable_primitives(stores);
    for &name in PDFTEX_PRIMITIVE_NAMES {
        let symbol = stores.intern(name);
        stores.register_primitive_meaning(name, stores.meaning(symbol));
    }
}

/// Reconstructs pdfTeX's original primitive table after a format load without
/// replacing live meanings restored from the format image.
pub(crate) fn register_pdftex_layer(stores: &mut Universe) {
    let mut pristine = Universe::default();
    tex_expand::install_etex_expandable_primitives(&mut pristine);
    install_pdftex_layer(&mut pristine);
    for &name in PDFTEX_PRIMITIVE_NAMES {
        stores.register_primitive_meaning(
            name,
            pristine
                .primitive_meaning(name)
                .expect("the pristine pdfTeX layer registers every inventory name"),
        );
    }
}

pub(crate) fn initialize_pdftex_parameter_defaults(stores: &mut Universe) {
    for &(parameter, value) in PDFTEX_INT_PARAMETER_DEFAULTS {
        stores.set_int_param_global(parameter, value);
    }
    for &(_, parameter, value) in PDFTEX_DIMEN_PARAMETERS {
        stores.set_dimen_param_global(parameter, Scaled::from_raw(value));
    }
    for &(_, parameter) in PDFTEX_TOK_PARAMETERS {
        stores.set_tok_param_global(parameter, TokenListId::EMPTY);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        prepare_etex_run_stores, prepare_latex_run_stores, prepare_pdftex_run_stores,
        prepare_run_stores,
    };
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::ExpandablePrimitive;
    use tex_state::meaning::MeaningFlags;
    use tex_state::token::{Catcode, Token};
    use tex_state::{
        FileModificationDate, JobClock, PdfDocumentFragmentKind, ShellEscapePolicy, World,
    };

    #[test]
    fn source_derived_inventory_is_the_exact_pinned_158_name_set() {
        let document = include_str!("../../../docs/pdftex_primitives.md");
        let table = document
            .split("| PDF token-list parameters")
            .nth(1)
            .expect("source checklist starts")
            .split("Counts in the table sum to 158")
            .next()
            .expect("source checklist ends");
        let mut source_names = table
            .split('`')
            .skip(1)
            .step_by(2)
            .filter_map(|quoted| quoted.strip_prefix('\\'))
            .collect::<Vec<_>>();
        source_names.sort_unstable();
        source_names.dedup();

        assert_eq!(PDFTEX_PRIMITIVE_NAMES.len(), 158);
        assert_eq!(
            PDFTEX_PRIMITIVE_NAMES
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            158,
            "the registered inventory must not contain duplicates",
        );
        assert_eq!(PDFTEX_PRIMITIVE_NAMES, source_names);
    }

    #[test]
    fn form_names_have_exact_append_only_identity() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        for (name, expected) in [
            ("pdfxform", UnexpandablePrimitive::PdfXForm),
            ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
        ] {
            let symbol = stores.intern(name);
            assert_eq!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(expected),
            );
        }
        assert_eq!(UnexpandablePrimitive::PdfXForm.operand(), 251);
        assert_eq!(UnexpandablePrimitive::PdfRefXForm.operand(), 252);
        assert_eq!(InternalInteger::PdfLastXForm.operand(), 16);
        assert_eq!(ExpandablePrimitive::PdfXFormName.operand(), 84);
    }

    #[test]
    fn pdfxform_consumes_box_and_captures_options_and_dimensions() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1",
                "\\setbox0=\\hbox to 10pt{}",
                "\\pdfxform attr {/A 1} resources {/R 2} 0",
                "\\message{last=\\the\\pdflastxform,name=\\pdfxformname\\pdflastxform}",
                "\\pdfrefxform 1\\end",
            ),
            &mut stores,
        )
        .expect("scan and reference a PDF form");
        assert!(stores.box_reg(0).is_none());
        let form = stores.pdf_form(1).expect("captured form");
        assert_eq!(form.resource(), 1);
        assert_eq!(form.width(), Scaled::from_raw(10 * 65_536));
        assert!(form.attr().is_some());
        assert!(form.resources().is_some());
        let output = crate::run_memory_with_stores(
            "\\message{name=\\pdfxformname1,last=\\the\\pdflastxform}\\end",
            &mut stores,
        )
        .expect("expand form enquiries");
        assert_eq!(output, " name=1,last=1");
    }

    #[test]
    fn pdfxform_rejects_void_boxes_and_dvi_mode() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores("\\pdfoutput=1\\pdfxform0\\end", &mut stores)
            .expect_err("void form box must fail");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): \\pdfxform cannot be used with a void box"
        );
        crate::run_memory_with_stores("\\setbox0=\\hbox{}\\pdfxform0\\end", &mut stores)
            .expect("form allocation continues after the failed reserved identity");
        let form = stores
            .pdf_form(3)
            .expect("second object and resource are retained");
        assert_eq!(form.resource(), 2);

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores("\\pdfxform0\\end", &mut stores)
            .expect_err("DVI mode must reject forms");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (\\pdfxform): not allowed in DVI mode (\\pdfoutput <= 0)."
        );
    }

    #[test]
    fn pdf_forms_rollback_and_replay_reuse_canonical_identity() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let snapshot = stores.snapshot();
        let source = "\\pdfoutput=1\\setbox0=\\hbox{}\\pdfxform0\\end";
        crate::run_memory_with_stores(source, &mut stores).expect("first form run");
        let first_hash = stores.testing_state_hash();
        assert_eq!(stores.pdf_last_form(), 1);
        stores.rollback(&snapshot);
        assert_eq!(stores.pdf_last_form(), 0);
        assert!(stores.pdf_forms().next().is_none());
        crate::run_memory_with_stores(source, &mut stores).expect("replayed form run");
        assert_eq!(stores.pdf_last_form(), 1);
        assert_eq!(stores.testing_state_hash(), first_hash);
    }

    #[test]
    fn pdf_form_state_and_diagnostics_match_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_form_state", "ref");
        let expected = [
            "initial=0",
            "h-form=1/1/131072,131072/void=yes",
            "v-form=3/0,262144",
            "math-form=5/65536,131072",
            "lazy-before=7/65536,131072",
            "lazy-after=7/4/196608,131072",
        ];
        for line in expected {
            assert!(
                reference.contains(line),
                "oracle missing {line:?}: {reference}"
            );
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_form_state.tex"),
            &mut stores,
        )
        .expect("execute pinned form-state fixture");
        let terminal = stores.world().memory_terminal_output().unwrap_or_default();
        let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
        for line in expected {
            assert!(
                observed.contains(line),
                "Umber missing {line:?}: {observed}"
            );
        }
        assert_eq!(
            stores
                .pdf_forms()
                .map(|form| (form.object(), form.resource()))
                .collect::<Vec<_>>(),
            [(1, 1), (3, 2), (5, 3), (7, 4)]
        );

        let diagnostic = test_support::read_fixture("tex_exec", "pdf_form_diagnostics", "ref");
        assert!(
            diagnostic.contains("pdfTeX error (ext1): \\pdfxform cannot be used with a void box.")
        );
        let traversal_diagnostic =
            test_support::read_fixture("tex_exec", "pdf_form_traversal_diagnostics", "ref");
        assert!(traversal_diagnostic.contains("1 unmatched \\pdfsave after form ship"));
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_form_diagnostics.tex"),
            &mut stores,
        )
        .expect_err("void form fixture must fail");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): \\pdfxform cannot be used with a void box"
        );
    }

    #[test]
    fn pdf_objects_reserve_initialize_reference_and_report_last_object() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1",
                "\\pdfobj reserveobjnum",
                "\\pdfobj useobjnum 1 stream attr {/Subtype /XML} {payload}",
                "\\pdfrefobj 1",
                "\\immediate\\pdfobj {42}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("execute raw PDF objects");

        assert_eq!(stores.pdf_last_object(), 2);
        let records = stores.pdf_raw_objects();
        assert_eq!(records.len(), 2);
        let first = records[0];
        let first_data = first.data().expect("initialized reserved object");
        assert_eq!(first.id().raw(), 1);
        assert!(first_data.is_stream());
        assert!(!first_data.is_file());
        assert!(first_data.stream_attr().is_some());
        assert!(first.is_referenced());
        assert!(!first.is_immediate());
        assert_eq!(records[1].id().raw(), 2);
        assert!(records[1].is_immediate());
    }

    #[test]
    fn pdf_accessibility_controls_scan_globally_and_reject_dvi_mode() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1",
                "\\def\\spacename{fixture}",
                "{\\pdfspacefont{\\spacename-space}}",
                "\\shipout\\hbox{a\\pdfinterwordspaceon b\\pdffakespace",
                "\\pdfinterwordspaceoff c}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("execute PDF accessibility controls");
        let page = stores.pdf_pages()[0];
        assert_eq!(
            stores.pdf_space_font_name(page.space_font_name_id()),
            Some(b"fixture-space".as_slice())
        );

        for primitive in [
            "\\pdfinterwordspaceon",
            "\\pdfinterwordspaceoff",
            "\\pdffakespace",
            "\\pdfspacefont{fixture}",
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let error = crate::run_memory_with_stores(
                &format!("\\pdfoutput=0{primitive}\\end"),
                &mut stores,
            )
            .expect_err("PDF-only accessibility primitive must fail in DVI mode");
            assert!(
                error.to_string().contains("not allowed in DVI mode"),
                "{primitive}: {error}"
            );
        }
    }

    #[test]
    fn pdf_annotations_and_links_allocate_pair_and_anchor_typed_effects() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1",
                "\\pdfannot reserveobjnum",
                "\\message{a=\\the\\pdflastannot/l=\\the\\pdflastlink}",
                "\\shipout\\hbox{",
                "\\pdfannot useobjnum 1 width 10pt {/Subtype /Text}",
                "\\pdfstartlink height 6pt attr{/Border [0 0 0]}",
                "user{/Subtype /Link /A << /S /URI /URI (u) >>}",
                "\\pdfrunninglinkoff X\\pdfrunninglinkon\\pdfendlink}",
                "\\message{A=\\the\\pdflastannot/L=\\the\\pdflastlink}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("annotation and link lifecycle");
        assert!(output.contains("A=1/L=2"), "{output}");
        assert_eq!(stores.pdf_annotations().len(), 1);
        assert_eq!(stores.pdf_links().len(), 1);
        assert!(stores.open_pdf_links().is_empty());

        let hash = stores.world().artifact_commits()[0];
        let bytes = stores
            .world()
            .read_artifact(hash)
            .expect("artifact read")
            .expect("artifact exists");
        let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
        assert_eq!(
            artifact
                .effects
                .iter()
                .filter_map(|effect| match effect {
                    tex_out::PageEffect::PdfAnnotation(marker) => Some(*marker),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![
                tex_out::PdfAnnotationEffect::Annotation { object: 1 },
                tex_out::PdfAnnotationEffect::LinkStart { object: 2 },
                tex_out::PdfAnnotationEffect::RunningLink(false),
                tex_out::PdfAnnotationEffect::RunningLink(true),
                tex_out::PdfAnnotationEffect::LinkEnd { object: 2 },
            ]
        );
    }

    #[test]
    fn pdf_link_level_mismatch_warns_and_closes_the_active_link() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1 X\\hbox{\\pdfstartlink user{/Subtype /Link}",
                "inside}\\pdfendlink\\end",
            ),
            &mut stores,
        )
        .expect("level mismatch is recoverable");
        assert!(
            output.contains(
                "pdfTeX warning: \\pdfendlink ended up in different nesting level than \\pdfstartlink"
            ),
            "{output}"
        );
        assert!(stores.open_pdf_links().is_empty());
    }

    #[test]
    fn pdf_objects_match_reference_errors_and_useobjnum_recovery() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            "\\pdfoutput=1\\pdfobj useobjnum 99 {fallback}\\message{last=\\the\\pdflastobj}\\end",
            &mut stores,
        )
        .expect("recover invalid useobjnum");
        assert_eq!(
            output,
            "\npdfTeX warning (\\pdfobj): invalid object number being ignored\nlast=1"
        );
        assert_eq!(stores.pdf_last_object(), 1);

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores("\\pdfoutput=1\\pdfrefobj 99\\end", &mut stores)
            .expect_err("invalid reference must be fatal");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): cannot find referenced object."
        );

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores(
            "\\pdfoutput=1\\immediate\\pdfobj reserveobjnum\\end",
            &mut stores,
        )
        .expect_err("immediate reservation must be fatal");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): `\\pdfobj reserveobjnum' cannot be used with \\immediate."
        );
    }

    #[test]
    fn pdfrefobj_is_applied_only_when_its_owning_list_ships() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            "\\pdfoutput=1\\pdfobj{x}\\setbox0=\\hbox{\\pdfrefobj 1}\\end",
            &mut stores,
        )
        .expect("discarded reference box executes");
        assert!(
            !stores
                .pdf_raw_object(1)
                .expect("raw object 1")
                .is_referenced()
        );

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1\\pdfobj{x}",
                "\\setbox0=\\hbox{\\pdfrefobj 1}\\shipout\\box0\\end",
            ),
            &mut stores,
        )
        .expect("shipped reference box executes");
        assert!(
            stores
                .pdf_raw_object(1)
                .expect("raw object 1")
                .is_referenced()
        );
    }

    #[test]
    fn pdf_document_fragments_expand_and_preserve_source_order() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        crate::run_memory_with_stores(
            concat!(
                "\\def\\value{one}",
                "\\pdfinfo{/First (\\value)}",
                "\\def\\value{two}",
                "\\pdfcatalog{/Catalog (\\value)}",
                "\\pdfinfo{/Second (\\value)}",
                "\\pdfnames{/Names (\\value)}",
                "\\pdftrailer{/Trailer (\\value)}",
                "\\pdftrailerid{<0123><4567>}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("execute document dictionary actions");

        let fragments = |kind| {
            stores
                .pdf_document_fragments(kind)
                .map(|tokens| token_list_text(&stores, tokens))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            fragments(PdfDocumentFragmentKind::Info),
            ["/First (one)", "/Second (two)"]
        );
        assert_eq!(
            fragments(PdfDocumentFragmentKind::Catalog),
            ["/Catalog (two)"]
        );
        assert_eq!(fragments(PdfDocumentFragmentKind::Names), ["/Names (two)"]);
        assert_eq!(
            fragments(PdfDocumentFragmentKind::Trailer),
            ["/Trailer (two)"]
        );
        assert_eq!(
            fragments(PdfDocumentFragmentKind::TrailerId),
            ["<0123><4567>"]
        );
    }

    #[test]
    fn pdf_document_fragments_match_dvi_mode_consumption() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            "\\pdfinfo{/Ignored true}\\message{continued}\\end",
            &mut stores,
        )
        .expect("warning form scans and ignores its argument");
        assert!(output.contains("pdfTeX warning (\\pdfinfo)"));
        assert!(output.contains("continued"));
        assert_eq!(
            stores
                .pdf_document_fragments(PdfDocumentFragmentKind::Info)
                .count(),
            0
        );

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let error = crate::run_memory_with_stores("\\pdfnames{/Forbidden true}\\end", &mut stores)
            .expect_err("pdfnames must fail before scanning in DVI mode");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (\\pdfnames): not allowed in DVI mode (\\pdfoutput <= 0)."
        );

        for (source, name) in [
            ("\\pdfobj{x}\\end", "pdfobj"),
            ("\\pdfrefobj 3\\end", "pdfrefobj"),
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let error = crate::run_memory_with_stores(source, &mut stores)
                .expect_err("object actions are forbidden in DVI mode");
            assert_eq!(
                error.to_string(),
                format!("pdfTeX error (\\{name}): not allowed in DVI mode (\\pdfoutput <= 0).")
            );
        }
    }

    #[test]
    fn pdfcatalog_openaction_scans_expanded_actions_and_rejects_duplicates() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\pdfoutput=1\\def\\view{/FitH 10}",
                "\\pdfcatalog{/PageMode /UseNone} openaction goto page 1 {\\view}",
                "\\end",
            ),
            &mut stores,
        )
        .expect("open action scans");
        let action = stores.pdf_catalog_open_action().expect("catalog action");
        assert_eq!(action.id(), 1);
        let tex_state::PdfActionSpec::GoTo(destination) = action.spec() else {
            panic!("expected GoTo action");
        };
        let tex_state::PdfActionTarget::Page { number, view } = destination.target else {
            panic!("expected page target");
        };
        assert_eq!(number, 1);
        assert_eq!(token_list_text(&stores, view), "/FitH 10");

        let error = crate::run_memory_with_stores(
            "\\pdfcatalog{} openaction user{<< /S /Named >>}\\end",
            &mut stores,
        )
        .expect_err("duplicate open action is fatal before rescanning");
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): duplicate of openaction"
        );
    }

    #[test]
    fn pdfcatalog_openaction_is_consumed_without_allocation_in_dvi_mode() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfcatalog{} openaction goto file{other.pdf} page 2 {/Fit} newwindow",
                "\\pdfcatalog{} openaction user{<< /S /Named /N /Print >>}",
                "\\message{continued}\\end",
            ),
            &mut stores,
        )
        .expect("DVI mode consumes repeated ignored open actions");
        assert!(output.contains("pdfTeX warning (\\pdfcatalog)"));
        assert!(output.contains("continued"));
        assert_eq!(stores.pdf_catalog_open_action(), None);

    }

    #[test]
    fn saved_position_and_snapping_names_have_exact_pdftex_identity() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        for (name, expected) in [
            ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
            ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
            ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
            ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
        ] {
            let symbol = stores.intern(name);
            assert_eq!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(expected)
            );
        }
        for (name, expected) in [
            ("pdflastxpos", InternalInteger::PdfLastXPos),
            ("pdflastypos", InternalInteger::PdfLastYPos),
        ] {
            let symbol = stores.intern(name);
            assert_eq!(stores.meaning(symbol), Meaning::InternalInteger(expected));
        }
        let nonexistent_alias = stores.intern("pdfsnaptorefpoint");
        assert_eq!(stores.meaning(nonexistent_alias), Meaning::Undefined);
    }

    #[test]
    fn pdftex_layer_is_visible_only_in_pdftex_mode() {
        for (prepare, intentional_overlaps) in [
            (prepare_run_stores as fn(&mut Universe), &[][..]),
            (
                prepare_etex_run_stores as fn(&mut Universe),
                &["ifincsname"][..],
            ),
            (
                prepare_latex_run_stores as fn(&mut Universe),
                &["expanded", "ifincsname"][..],
            ),
        ] {
            let mut stores = Universe::default();
            prepare(&mut stores);
            for &name in PDFTEX_PRIMITIVE_NAMES {
                if intentional_overlaps.contains(&name) {
                    continue;
                }
                let symbol = stores.intern(name);
                assert_eq!(stores.meaning(symbol), Meaning::Undefined, "{name}");
            }
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        for &name in PDFTEX_PRIMITIVE_NAMES {
            let symbol = stores.intern(name);
            assert_ne!(stores.meaning(symbol), Meaning::Undefined, "{name}");
        }
        let revision = stores.intern("pdftexrevision");
        assert_eq!(
            stores.meaning(revision),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXRevision),
        );
    }

    #[test]
    fn pdftex_version_identity_matches_the_pinned_release() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\the\\pdftexversion\\pdftexrevision|\\pdftexbanner%",
        ));
        let mut output = String::new();
        let mut context = tex_state::ExpansionContext::new(&mut stores);
        while let Some(token) =
            tex_expand::get_x_token(&mut input, &mut context).expect("identity expansion")
        {
            let Token::Char { ch, .. } = tex_expand::semantic_token(token) else {
                panic!("identity emitted non-character token {token:?}");
            };
            output.push(ch);
        }
        assert_eq!(
            output,
            "140.27|This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025)",
        );
    }

    #[test]
    fn pdftex_identity_expansion_uses_pdftex_character_catcodes() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\the\\pdftexversion\\pdftexrevision|\\pdftexbanner%",
        ));
        let mut context = tex_state::ExpansionContext::new(&mut stores);
        while let Some(token) =
            tex_expand::get_x_token(&mut input, &mut context).expect("identity expansion")
        {
            match tex_expand::semantic_token(token) {
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                }
                | Token::Char {
                    cat: Catcode::Other,
                    ..
                } => {}
                token => panic!("identity emitted a non-pdfTeX character token {token:?}"),
            }
        }
    }

    fn expand_pdftex_characters(
        source: &str,
        configure: impl FnOnce(&mut Universe),
    ) -> Vec<(u8, Catcode)> {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        configure(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut context = tex_state::ExpansionContext::new(&mut stores);
        let mut output = Vec::new();
        while let Some(token) =
            tex_expand::get_x_token(&mut input, &mut context).expect("pdfTeX string expansion")
        {
            let Token::Char { ch, cat } = tex_expand::semantic_token(token) else {
                panic!("string primitive emitted non-character token {token:?}");
            };
            output.push((u8::try_from(u32::from(ch)).expect("pdfTeX byte token"), cat));
        }
        output
    }

    fn pdftex_bytes(source: &str) -> Vec<u8> {
        expand_pdftex_characters(source, |_| {})
            .into_iter()
            .map(|(byte, _)| byte)
            .collect()
    }

    #[test]
    fn pdf_color_stack_init_is_expandable_global_and_scans_both_page_keywords() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(concat!(
            "\\pdfcolorstackinit{A}|",
            "\\pdfcolorstackinit page direct{B}|",
            "\\pdfcolorstackinit page page{C}%",
        )));
        let mut context = tex_state::ExpansionContext::new(&mut stores);
        let mut output = String::new();
        while let Some(token) =
            tex_expand::get_x_token(&mut input, &mut context).expect("color stack expansion")
        {
            let Token::Char { ch, .. } = tex_expand::semantic_token(token) else {
                panic!("color stack init emitted non-character token");
            };
            output.push(ch);
        }
        assert_eq!(output, "1|2|3");
        assert_eq!(
            stores
                .apply_pdf_color_stack(
                    3,
                    tex_state::PdfColorStackTarget::Page,
                    &tex_state::PdfColorStackAction::Current,
                )
                .expect("allocated stack")
                .mode,
            tex_state::PdfColorStackMode::Page,
        );
    }

    #[test]
    fn pdftex_string_escapes_match_the_pinned_byte_oracle() {
        assert_eq!(
            pdftex_bytes("\\pdfescapestring{Text (1)\\pdfunescapehex{5C7F80}}%"),
            b"Text\\040\\(1\\)\\\\\\177\\200"
        );
        assert_eq!(
            pdftex_bytes("\\pdfescapename{Text \\pdfunescapehex{28292F23255B5D7B7D007F80FF}}%"),
            b"Text#20#28#29#2F#23#25#5B#5D#7B#7D#7F#80#FF"
        );
        assert_eq!(
            pdftex_bytes("\\pdfescapehex{Az \\pdfunescapehex{007F80FF}}%"),
            b"417A20007F80FF"
        );
    }

    #[test]
    fn pdftex_unescapehex_ignores_junk_and_pads_an_odd_nibble() {
        assert_eq!(
            pdftex_bytes("\\pdfescapehex{\\pdfunescapehex{4g1!42zF}}%"),
            b"4142F0"
        );
    }

    #[test]
    fn pdftex_string_results_use_space_and_other_catcodes() {
        for (byte, catcode) in expand_pdftex_characters("\\pdfunescapehex{20412800FF}%", |_| {}) {
            assert_eq!(
                catcode,
                if byte == b' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                },
                "byte {byte:02X}"
            );
        }
    }

    #[test]
    fn pdftex_string_scanning_expands_macros_and_spells_control_sequences() {
        assert_eq!(
            expand_pdftex_characters(
                "\\pdfescapehex{\\value\\noexpand\\foobar\\noexpand\\!}%",
                |stores| {
                    let value = stores.intern("value");
                    let body = stores.intern_token_list(&[
                        Token::Char {
                            ch: 'A',
                            cat: Catcode::Letter,
                        },
                        Token::Char {
                            ch: 'z',
                            cat: Catcode::Letter,
                        },
                    ]);
                    stores.set_macro_meaning(
                        value,
                        MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, body),
                    );
                },
            )
            .into_iter()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>(),
            b"417A5C666F6F626172205C21"
        );
        assert_eq!(
            expand_pdftex_characters(
                "\\pdfescapehex{\\noexpand\\foobar\\noexpand\\!}%",
                |stores| stores.set_int_param_global(IntParam::ESCAPE_CHAR, -1),
            )
            .into_iter()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>(),
            b"666F6F6261722021"
        );
    }

    #[test]
    fn pdfstrcmp_uses_unsigned_pdftex_byte_ordering() {
        assert_eq!(
            pdftex_bytes(concat!(
                "\\pdfstrcmp{a}{aa},",
                "\\pdfstrcmp{aa}{a},",
                "\\pdfstrcmp{same}{same},",
                "\\pdfstrcmp{\\pdfunescapehex{80}}{\\pdfunescapehex{7F}},",
                "\\pdfstrcmp{\\noexpand\\a}{\\noexpand\\b}%",
            )),
            b"-1,1,0,1,-1"
        );

        let mut pdftex = Universe::default();
        prepare_pdftex_run_stores(&mut pdftex);
        let pdfstrcmp = pdftex.intern("pdfstrcmp");
        assert_eq!(
            pdftex.meaning(pdfstrcmp),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare)
        );
        let mut latex = Universe::default();
        prepare_latex_run_stores(&mut latex);
        let strcmp = latex.intern("strcmp");
        assert_eq!(latex.meaning(strcmp), pdftex.meaning(pdfstrcmp));
    }

    #[test]
    fn pdfmatch_reports_posix_leftmost_longest_captures() {
        assert_eq!(
            pdftex_bytes(concat!(
                "\\pdfmatch{(a+)(b*)}{xxaaabbzz}|",
                "\\pdflastmatch0|\\pdflastmatch1|\\pdflastmatch2|\\pdflastmatch3|",
                "\\pdfmatch{a|aa}{xaa}|\\pdflastmatch0|",
                "\\pdfmatch icase{abc}{xAbCy}|\\pdflastmatch0%",
            )),
            b"1|2->aaabb|2->aaa|5->bb|-1->|1|1->aa|1|1->AbC"
        );
    }

    #[test]
    fn pdfmatch_subcount_and_no_match_follow_the_pinned_oracle() {
        assert_eq!(
            pdftex_bytes(concat!(
                "\\pdfmatch subcount 0{(a)}{a}|\\pdflastmatch0|",
                "\\pdfmatch subcount 2{(a)(b)(c)}{abc}|",
                "\\pdflastmatch0|\\pdflastmatch1|\\pdflastmatch2|",
                "\\pdfmatch{z}{abc}|\\pdflastmatch0%",
            )),
            b"1|-1->|1|0->abc|0->a|-1->|0|-1->"
        );
    }

    #[test]
    fn pdfmatch_uses_c_string_nul_termination() {
        assert_eq!(
            pdftex_bytes(concat!(
                "\\pdfmatch{ab\\pdfunescapehex{00}z}{xxabyy}|\\pdflastmatch0|",
                "\\pdfmatch{ab}{xxab\\pdfunescapehex{00}ab}|\\pdflastmatch0%",
            )),
            b"1|2->ab|1|2->ab"
        );
    }

    #[test]
    fn pdfmatch_state_is_global_and_compile_failures_preserve_it() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfmatch{(a)}{xa}\\message{before=\\pdflastmatch1} ",
                "{\\pdfmatch{(b)}{yb}}\\message{group=\\pdflastmatch1} ",
                "\\pdfmatch{[}{q}\\message{bad=\\pdflastmatch1} ",
                "\\message{negative=\\pdflastmatch-2}\\end",
            ),
            &mut stores,
        )
        .expect("recover pdfTeX regex diagnostics");
        assert!(output.contains("before=1->a"), "{output}");
        assert!(output.contains("group=1->b"), "{output}");
        assert!(output.contains("brackets ([ ]) not balanced"), "{output}");
        assert!(output.contains("bad=1->b"), "{output}");
        assert!(output.contains("Bad match number (-2)."), "{output}");
        assert!(output.contains("negative=1->b"), "{output}");
    }

    #[test]
    fn pdftex_random_primitives_match_seeded_reference_sequence() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfsetrandomseed 1 ",
                "\\message{seed=\\the\\pdfrandomseed}",
                "\\message{u0=\\pdfuniformdeviate0}",
                "\\message{u1=\\pdfuniformdeviate1}",
                "\\message{u2=\\pdfuniformdeviate2}",
                "\\message{u10a=\\pdfuniformdeviate10}",
                "\\message{u10b=\\pdfuniformdeviate10}",
                "\\message{uneg=\\pdfuniformdeviate-10}",
                "\\message{n1=\\pdfnormaldeviate}",
                "\\message{n2=\\pdfnormaldeviate}",
                "\\pdfsetrandomseed -1 ",
                "\\message{negative-seed=\\the\\pdfrandomseed}",
                "\\message{repeat=\\pdfuniformdeviate10}\\end",
            ),
            &mut stores,
        )
        .expect("seeded pdfTeX random sequence");
        for expected in [
            "seed=1",
            "u0=0",
            "u1=0",
            "u2=1",
            "u10a=6",
            "u10b=5",
            "uneg=-4",
            "n1=44619",
            "n2=31254",
            "negative-seed=1",
            "repeat=7",
        ] {
            assert!(output.contains(expected), "{expected}: {output}");
        }
    }

    #[test]
    fn pdftex_timer_reset_and_shell_status_use_world_inputs() {
        let mut stores = Universe::default();
        stores.world_mut().set_pdf_time_micros(1_250_000);
        stores
            .world_mut()
            .set_shell_escape_policy(ShellEscapePolicy::Restricted);
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\message{elapsed=\\the\\pdfelapsedtime}",
                "\\message{shell=\\the\\pdfshellescape}",
                "\\pdfresettimer",
                "\\message{reset=\\the\\pdfelapsedtime}\\end",
            ),
            &mut stores,
        )
        .expect("pdfTeX timer and shell enquiries");
        assert!(output.contains("elapsed=81920"), "{output}");
        assert!(output.contains("shell=2"), "{output}");
        assert!(output.contains("reset=0"), "{output}");
    }

    #[test]
    fn pdftex_utility_format_load_uses_the_new_world_session_inputs() {
        let mut source = Universe::default();
        prepare_pdftex_run_stores(&mut source);
        source.world_mut().set_pdf_random_seed(1);
        source.world_mut().set_pdf_time_micros(1_000_000);
        source.world_mut().reset_pdf_timer();
        let format = source.dump_format().expect("utility-free format image");

        let mut world = World::memory();
        world.set_pdf_random_seed(9);
        world.set_pdf_time_micros(2_000_000);
        world.set_shell_escape_policy(ShellEscapePolicy::Enabled);
        let mut loaded = Universe::from_format(world, &format).expect("load with fresh World");
        crate::install_pdftex_format_primitives(&mut loaded);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\message{seed=\\the\\pdfrandomseed}",
                "\\message{elapsed=\\the\\pdfelapsedtime}",
                "\\message{shell=\\the\\pdfshellescape}\\end",
            ),
            &mut loaded,
        )
        .expect("fresh World utility inputs");
        assert!(output.contains("seed=9"), "{output}");
        assert!(output.contains("elapsed=131072"), "{output}");
        assert!(output.contains("shell=1"), "{output}");
    }

    #[test]
    fn pdftex_random_scanners_report_and_recover_bounds() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfsetrandomseed 999999999999 ",
                "\\message{seed=\\the\\pdfrandomseed}",
                "\\message{missing=\\pdfuniformdeviate\\relax}\\end",
            ),
            &mut stores,
        )
        .expect("recover random scanner diagnostics");
        assert!(output.contains("Number too big"), "{output}");
        assert!(output.contains("seed=2147483647"), "{output}");
        assert!(output.contains("Missing number"), "{output}");
        assert!(output.contains("missing=0"), "{output}");
    }

    fn seed_pdftex_file_facts(stores: &mut Universe) {
        stores
            .world_mut()
            .set_memory_file("asset.bin", vec![0x00, 0x41, 0x7f, 0x80, 0xff, 0x0a])
            .expect("seed virtual file");
        stores
            .world_mut()
            .set_memory_file_modification_date(
                "asset.bin",
                FileModificationDate::with_offset(
                    JobClock {
                        time: 23 * 60 + 5,
                        second: 6,
                        day: 2,
                        month: 2,
                        year: 2024,
                    },
                    -5 * 60,
                ),
            )
            .expect("seed virtual modification date");
    }

    #[test]
    fn pdftex_virtual_file_enquiries_match_the_pinned_oracle() {
        let output = expand_pdftex_characters(
            concat!(
                "\\pdfcreationdate|",
                "\\pdffilemoddate{asset.bin}|",
                "\\pdffilesize{asset.bin}|",
                "\\pdfmdfivesum{abc}|",
                "\\pdfmdfivesum{}|",
                "\\pdfmdfivesum file {asset.bin}%",
            ),
            seed_pdftex_file_facts,
        )
        .into_iter()
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
        assert_eq!(
            output,
            concat!(
                "D:19700101000000Z|",
                "D:20240202230506-05'00'|",
                "6|",
                "900150983CD24FB0D6963F7D28E17F72|",
                "D41D8CD98F00B204E9800998ECF8427E|",
                "533D621634EC926267C997E4FADE6938",
            )
            .as_bytes()
        );
    }

    #[test]
    fn pdffiledump_matches_pdftex_offset_and_length_boundaries() {
        let output = expand_pdftex_characters(
            concat!(
                "[\\pdffiledump{asset.bin}]|",
                "[\\pdffiledump length 0 {asset.bin}]|",
                "[\\pdffiledump length 3 {asset.bin}]|",
                "[\\pdffiledump offset 2 {asset.bin}]|",
                "[\\pdffiledump offset 2 length 2 {asset.bin}]|",
                "[\\pdffiledump offset 99 length 2 {asset.bin}]|",
                "[\\pdffiledump offset 2 length 99 {asset.bin}]%",
            ),
            seed_pdftex_file_facts,
        )
        .into_iter()
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
        assert_eq!(output, b"[]|[]|[00417F]|[]|[7F80]|[]|[7F80FF0A]");
    }

    #[test]
    fn pdftex_missing_virtual_file_enquiries_expand_to_nothing() {
        let output = expand_pdftex_characters(
            concat!(
                "[\\pdffilemoddate{missing}]|",
                "[\\pdffilesize{missing}]|",
                "[\\pdfmdfivesum file {missing}]|",
                "[\\pdffiledump length 2 {missing}]%",
            ),
            |_| {},
        )
        .into_iter()
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
        assert_eq!(output, b"[]|[]|[]|[]");
    }

    #[test]
    fn pdffiledump_reports_and_recovers_negative_ranges() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        seed_pdftex_file_facts(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\message{O=[\\pdffiledump offset -1 length 2 {asset.bin}]} ",
                "\\message{L=[\\pdffiledump offset 1 length -2 {asset.bin}]}\\end",
            ),
            &mut stores,
        )
        .expect("recover negative dump ranges");
        assert!(output.contains("! Bad file offset (-1)."), "{output}");
        assert!(output.contains("! Bad dump length (-2)."), "{output}");
        assert!(output.contains("O=[0041]"), "{output}");
        assert!(output.contains("L=[]"), "{output}");
    }

    #[test]
    fn primitive_identity_and_absolute_conditionals_match_pdftex() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\ifpdfprimitive\\count\\message{count-original}\\else\\message{count-bad}\\fi ",
                "\\let\\countalias=\\count ",
                "\\ifpdfprimitive\\countalias\\message{alias-bad}\\else\\message{alias-false}\\fi ",
                "\\ifpdfprimitive\\undefinedname\\message{undefined-bad}\\else\\message{undefined-false}\\fi ",
                "{\\def\\count{shadow}",
                "\\ifpdfprimitive\\count\\message{shadow-bad}\\else\\message{shadow-false}\\fi ",
                "\\pdfprimitive\\count0=12\\message{local-count=\\the\\pdfprimitive\\count0}}",
                "\\pdfprimitive\\count0=37 ",
                "\\ifpdfprimitive\\count\\message{restored}\\else\\message{restore-bad}\\fi ",
                "\\def\\pdftexrevision{shadow-revision}",
                "\\edef\\result{A\\pdfprimitive\\pdftexrevision B\\pdfprimitive\\undefinedname C}",
                "\\message{result=\\result/count=\\the\\count0} ",
                "\\ifpdfabsnum -3>2\\message{num-gt}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsnum 2<-3\\message{num-lt}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsnum -3=3\\message{num-eq}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsdim -3pt>2pt\\message{dim-gt}\\else\\message{dim-bad}\\fi ",
                "\\ifpdfabsdim 2pt<-3pt\\message{dim-lt}\\else\\message{dim-bad}\\fi ",
                "\\ifpdfabsdim -3pt=3pt\\message{dim-eq}\\else\\message{dim-bad}\\fi ",
                "\\end",
            ),
            &mut stores,
        )
        .expect("pdfTeX primitive utility execution");

        for marker in [
            "count-original",
            "alias-false",
            "undefined-false",
            "shadow-false",
            "restored",
            "local-count=12",
            "result=A.27BC/count=37",
            "num-gt",
            "num-lt",
            "num-eq",
            "dim-gt",
            "dim-lt",
            "dim-eq",
        ] {
            assert!(output.contains(marker), "missing {marker}: {output}");
        }
        assert!(!output.contains("-bad"), "{output}");
    }

    #[test]
    fn primitive_registry_reconstructs_after_format_load_without_unshadowing() {
        let mut source = Universe::default();
        prepare_pdftex_run_stores(&mut source);
        let count = source.intern("count");
        let revision = source.intern("pdftexrevision");
        source.set_meaning(count, Meaning::Relax);
        source.set_meaning(revision, Meaning::Relax);
        let format = source.dump_format().expect("dump shadowed format");
        let mut loaded = Universe::from_format(World::default(), &format).expect("load format");
        crate::install_pdftex_format_primitives(&mut loaded);

        let output = crate::run_memory_with_stores(
            concat!(
                "\\ifpdfprimitive\\count\\message{count-bad}\\else\\message{count-shadowed}\\fi ",
                "\\pdfprimitive\\count0=41 ",
                "\\edef\\x{\\pdfprimitive\\pdftexrevision}",
                "\\message{x=\\x/count=\\the\\pdfprimitive\\count0}\\end",
            ),
            &mut loaded,
        )
        .expect("run restored primitive registry");
        assert!(output.contains("count-shadowed"), "{output}");
        assert!(output.contains("x=.27/count=41"), "{output}");
        assert!(!output.contains("count-bad"), "{output}");
    }

    #[test]
    fn pdftex_parameter_defaults_match_the_pinned_initex_engine() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);

        assert_eq!(PDFTEX_INT_PARAMETER_MEANINGS.len(), 38);
        assert_eq!(PDFTEX_INT_PARAMETER_DEFAULTS.len(), 37);
        assert_eq!(PDFTEX_DIMEN_PARAMETERS.len(), 13);
        assert_eq!(PDFTEX_TOK_PARAMETERS.len(), 4);
        for &(parameter, expected) in PDFTEX_INT_PARAMETER_DEFAULTS {
            assert_eq!(stores.int_param(parameter), expected, "{parameter:?}");
        }
        for &(name, parameter, expected) in PDFTEX_DIMEN_PARAMETERS {
            assert_eq!(
                stores.dimen_param(parameter).raw(),
                expected,
                "{name} default"
            );
        }
        for &(name, parameter) in PDFTEX_TOK_PARAMETERS {
            assert_eq!(stores.tok_param(parameter), TokenListId::EMPTY, "{name}");
        }

        let alias = stores.intern("pdfoptionpdfminorversion");
        let canonical = stores.intern("pdfminorversion");
        assert_eq!(stores.meaning(alias), stores.meaning(canonical));
        for (obsolete, current) in [
            ("pdfoptionalwaysusepdfpagebox", "pdfforcepagebox"),
            ("pdfoptionpdfinclusionerrorlevel", "pdfinclusionerrorlevel"),
        ] {
            let obsolete = stores.intern(obsolete);
            let current = stores.intern(current);
            assert_ne!(stores.meaning(obsolete), stores.meaning(current));
        }
    }

    #[test]
    fn pdftex_parameter_defaults_are_not_installed_in_other_modes() {
        for prepare in [
            prepare_run_stores as fn(&mut Universe),
            prepare_etex_run_stores,
            prepare_latex_run_stores,
        ] {
            let mut stores = Universe::default();
            prepare(&mut stores);
            for &(parameter, _) in PDFTEX_INT_PARAMETER_DEFAULTS {
                assert_eq!(stores.int_param(parameter), 0, "{parameter:?}");
            }
            for &(_, parameter, _) in PDFTEX_DIMEN_PARAMETERS {
                assert_eq!(
                    stores.dimen_param(parameter),
                    Scaled::from_raw(0),
                    "{parameter:?}"
                );
            }
            for &(_, parameter) in PDFTEX_TOK_PARAMETERS {
                assert_eq!(stores.tok_param(parameter), TokenListId::EMPTY);
            }
        }
    }

    #[test]
    fn pdftex_parameters_obey_groups_globaldefs_and_legacy_aliases() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\pdfcompresslevel=7 ",
                "\\pdfhorigin=10pt ",
                "\\pdfpagesattr{outer} ",
                "{\\pdfcompresslevel=3 ",
                "\\pdfhorigin=20pt ",
                "\\pdfpagesattr{inner} ",
                "\\message{local=\\the\\pdfcompresslevel/\\the\\pdfhorigin/\\the\\pdfpagesattr}} ",
                "\\message{restored=\\the\\pdfcompresslevel/\\the\\pdfhorigin/\\the\\pdfpagesattr} ",
                "{\\globaldefs=1 ",
                "\\pdfcompresslevel=4 ",
                "\\pdfhorigin=30pt ",
                "\\pdfpagesattr{global}} ",
                "\\pdfoptionpdfminorversion=7 ",
                "\\pdfoptionalwaysusepdfpagebox=2 ",
                "\\pdfoptionpdfinclusionerrorlevel=1 ",
                "{\\pdfoptionpdfminorversion=6 ",
                "\\pdfoptionalwaysusepdfpagebox=4 ",
                "\\pdfoptionpdfinclusionerrorlevel=3 ",
                "\\message{compat-local=\\the\\pdfminorversion/\\the\\pdfoptionalwaysusepdfpagebox/\\the\\pdfforcepagebox/\\the\\pdfoptionpdfinclusionerrorlevel/\\the\\pdfinclusionerrorlevel}} ",
                "\\message{compat-restored=\\the\\pdfminorversion/\\the\\pdfoptionalwaysusepdfpagebox/\\the\\pdfforcepagebox/\\the\\pdfoptionpdfinclusionerrorlevel/\\the\\pdfinclusionerrorlevel} ",
                "\\end",
            ),
            &mut stores,
        )
        .expect("pdfTeX parameter assignments");

        assert!(output.contains("local=3/20.0pt/inner"), "{output}");
        assert!(output.contains("restored=7/10.0pt/outer"), "{output}");
        assert!(output.contains("compat-local=6/4/0/3/0"), "{output}");
        assert!(output.contains("compat-restored=7/2/0/1/0"), "{output}");
        assert_eq!(stores.int_param(IntParam::PDF_COMPRESS_LEVEL), 4);
        assert_eq!(stores.int_param(IntParam::PDF_MINOR_VERSION), 7);
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            2
        );
        assert_eq!(stores.int_param(IntParam::PDF_FORCE_PAGE_BOX), 0);
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
            1
        );
        assert_eq!(stores.int_param(IntParam::PDF_INCLUSION_ERROR_LEVEL), 0);
        assert_eq!(
            stores.dimen_param(DimenParam::PDF_H_ORIGIN),
            Scaled::from_raw(30 * 65_536)
        );
        assert_eq!(
            token_list_text(&stores, stores.tok_param(TokParam::PDF_PAGES_ATTR)),
            "global"
        );
    }

    #[test]
    fn pdf_image_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_image_config", "ref");
        for expected in [
            "defaults=72/0/0/0/0/0/0/0/1000/2200/1/0/0/0",
            "local=-1/9000/-2/5/1/2/3/4/-3/1000001/2/-1/2/-1",
            "restored=96/300/1/2/3/4/1/2/900/1800/0/1/0/1",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_image_config.tex"),
            &mut stores,
        )
        .expect("pdfTeX image configuration assignments");
        for expected in [
            "defaults=72/0/0/0/0/0/0/0/1000/2200/1/0/0/0",
            "local=-1/9000/-2/5/1/2/3/4/-3/1000001/2/-1/2/-1",
            "restored=96/300/1/2/3/4/1/2/900/1800/0/1/0/1",
        ] {
            assert!(output.contains(expected), "missing {expected:?}: {output}");
        }
    }

    #[test]
    fn pdf_font_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_font_config", "ref");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/300/9",
            ".\\a A",
            ".\\b B",
            ".\\a (cmr10) A",
            ".\\b (cmr10@12.0pt) B",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        let mut stores = Universe::default();
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("seed cmr10");
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_font_config.tex"),
            &mut stores,
        )
        .expect("pdfTeX font configuration assignments and diagnostics");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/300/9",
            ".\\a A",
            ".\\b B",
            ".\\a (cmr10) A",
            ".\\b (cmr10@12.0pt) B",
        ] {
            assert!(output.contains(expected), "missing {expected:?}: {output}");
        }
        let configuration = stores.pdf_font_configuration();
        assert_eq!(configuration.resolved_pk_resolution(600), 300);
        assert!(configuration.traces_fonts());
        assert!(configuration.omits_charset());
    }

    #[test]
    fn pdf_microtype_effects_match_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_microtype_effects", "ref");
        for expected in [
            "\\kern 1.0 (for \\pdfprependkern/\\pdfappendkern)",
            "\\kern 5.0 (for \\pdfprependkern/\\pdfappendkern)",
            "\\glue 4.33333 plus 3.66666 minus 4.11111",
            "\\kern-1.0 (left margin)",
            "\\kern-2.0 (right margin)",
            "\\f (-50) A",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        let mut stores = Universe::default();
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("seed cmr10");
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_microtype_effects.tex"),
            &mut stores,
        )
        .expect("pdfTeX microtype effect fixture");
        for expected in [
            "> \\box0=\n\\hbox(6.83331+0.0)x14.58337\n.\\f A\n.\\f B",
            "> \\box3=\n\\hbox(6.83331+0.0)x24.58337",
            ".\\kern 5.0 (for \\pdfprependkern/\\pdfappendkern)",
            "> \\box4=\n\\hbox(6.83331+0.0)x14.58337",
            "> \\box6=\n\\hbox(6.83331+0.0)x18.9167",
            ".\\glue 4.33333 plus 3.66666 minus 4.11111",
            "> \\box7=\n\\hbox(6.83331+0.0)x17.9167",
            "..\\kern-1.0 (left margin)",
            "..\\kern-2.0 (right margin)",
            "> \\box10=\n\\vbox(6.83331+0.0)x20.0",
            "> \\box12=\n\\vbox(6.83331+0.0)x15.0",
            "..\\f (-50) A",
            "> \\box13=\n\\vbox(6.83331+0.0)x15.0",
        ] {
            assert!(output.contains(expected), "missing {expected:?}: {output}");
        }
    }

    #[test]
    fn pdf_font_codes_size_and_ligature_suppression_match_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_font_codes", "ref");
        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        let mut stores = Universe::default();
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("seed cmr10");
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_font_codes.tex"),
            &mut stores,
        )
        .expect("pdfTeX font-code fixture");
        for expected in [
            "defaults=0/0/1000/0/0/0/0/0/12.0pt",
            "assigned=7/-1000/800/1000/-1000/321/-432/543",
            "tag-before=1",
            "tag-after=0",
            ".\\a f",
            ".\\a i",
        ] {
            assert!(
                reference.contains(expected),
                "oracle missing {expected:?}: {reference}"
            );
            assert!(
                output.contains(expected),
                "Umber missing {expected:?}: {output}"
            );
        }
        assert!(
            !output.contains("ligature fi"),
            "ligature survived: {output}"
        );
    }

    #[test]
    fn pdf_output_policy_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_output_policy", "ref");
        for expected in [
            "defaults=0/1.4/9/0/3",
            "local=3/6 restored=7/5",
            "pdfTeX error (invalid pdfmajorversion)",
            "pdfTeX error (invalid pdfminorversion)",
            "Object streams disabled now",
            "recovered=1.4",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_output_policy.tex"),
            &mut stores,
        )
        .expect("Umber recovers from the pinned range cases");
        let terminal = stores.world().memory_terminal_output().unwrap_or_default();
        let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
        for expected in [
            "defaults=0/1.4/9/0/3",
            "local=3/6",
            "restored=7/5",
            "pdfTeX error (invalid pdfmajorversion)",
            "pdfTeX error (invalid pdfminorversion)",
            "Object streams disabled now",
            "recovered=1.4",
        ] {
            assert!(
                observed.contains(expected),
                "missing {expected:?}: {observed}"
            );
        }
        assert_eq!(
            stores.fixed_pdf_output_parameters(),
            Some(tex_state::PdfOutputParameters {
                output: 1,
                major_version: 1,
                minor_version: 4,
                compress_level: 7,
                object_compress_level: 0,
                decimal_digits: 4,
                gamma: 1_000,
                image_gamma: 2_200,
                image_hicolor: 1,
                image_apply_gamma: 0,
                draft_mode: 0,
                inclusion_copy_fonts: 0,
                pk_resolution: 0,
                unique_resource_names: 0,
            })
        );
    }

    #[test]
    fn pdf_insert_height_reads_live_page_insertion_accounting() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            concat!(
                "\\vsize=100pt ",
                "\\count254=1000 \\dimen254=100pt \\skip254=0pt ",
                "\\message{before=\\pdfinsertht254/absent=\\pdfinsertht253} ",
                "{\\insert254{\\hbox{\\vrule height10pt depth2pt width0pt}}} ",
                "\\message{first=\\pdfinsertht254} ",
                "\\insert254{\\hbox{\\vrule height3pt depth1pt width0pt}} ",
                "\\message{second=\\pdfinsertht254/absent=\\pdfinsertht253}",
            ),
            &mut stores,
        )
        .expect("pdfTeX insertion-height enquiry");

        for expected in [
            "before=0pt/absent=0pt",
            "first=12.0pt",
            "second=16.0pt/absent=0pt",
        ] {
            assert!(output.contains(expected), "missing {expected:?}: {output}");
        }
        assert_eq!(
            stores.page_insertion_height(254),
            Some(Scaled::from_raw(16 * Scaled::UNITY))
        );

        crate::run_memory_with_stores("\\end", &mut stores)
            .expect("finish the page containing insertions");
        assert_eq!(stores.page_insertion_height(254), None);

        let mut split_stores = Universe::default();
        prepare_pdftex_run_stores(&mut split_stores);
        let split_output = crate::run_memory_with_stores(
            concat!(
                "\\vsize=100pt ",
                "\\count254=1000 \\dimen254=5pt \\skip254=0pt ",
                "\\splittopskip=0pt \\splitmaxdepth=0pt ",
                "\\insert254{",
                "\\hbox{\\vrule height4pt depth0pt width0pt}",
                "\\vskip1pt",
                "\\hbox{\\vrule height4pt depth0pt width0pt}} ",
                "\\message{split=\\pdfinsertht254}",
            ),
            &mut split_stores,
        )
        .expect("split pdfTeX insertion-height enquiry");
        assert!(
            split_output.contains("split=4.0pt"),
            "split oracle mismatch: {split_output}"
        );
        assert_eq!(
            split_stores.page_insertion_height(254),
            Some(Scaled::from_raw(4 * Scaled::UNITY))
        );
    }

    #[test]
    fn pdf_ximage_bbox_matches_page_box_indices_raster_and_catcodes() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let pdf_id = tex_state::PdfExternalImageId::new(7).expect("PDF image id");
        stores
            .register_pdf_external_image(
                pdf_id,
                tex_state::PdfExternalImageMetadata::PdfPage {
                    page_box: tex_state::PdfPageBox {
                        left: Scaled::from_raw(0),
                        bottom: Scaled::from_raw(0),
                        right: Scaled::from_raw(655_384),
                        top: Scaled::from_raw(327_659),
                    },
                },
            )
            .expect("register PDF page-box metadata");
        stores
            .register_pdf_external_image(
                tex_state::PdfExternalImageId::new(8).expect("raster image id"),
                tex_state::PdfExternalImageMetadata::Raster,
            )
            .expect("register raster metadata");

        let output = crate::run_memory_with_stores(
            concat!(
                "\\message{bbox=[\\pdfximagebbox7 1]/[\\pdfximagebbox7 2]/",
                "[\\pdfximagebbox7 3]/[\\pdfximagebbox7 4]}",
                "\\message{raster=[\\pdfximagebbox8 1]/[\\pdfximagebbox8 4]}",
            ),
            &mut stores,
        )
        .expect("pdfTeX image bounding-box enquiries");
        assert!(
            output.contains("bbox=[0.0pt]/[0.0pt]/[10.00037pt]/[4.99968pt]"),
            "{output}"
        );
        assert!(output.contains("raster=[0.0pt]/[0.0pt]"), "{output}");

        let mut input = InputStack::new(MemoryInput::new("\\pdfximagebbox7 3"));
        let mut context = tex_state::ExpansionContext::new(&mut stores);
        let mut expanded = Vec::new();
        while let Some(token) =
            tex_expand::get_x_token(&mut input, &mut context).expect("bbox expansion")
        {
            expanded.push(tex_expand::semantic_token(token));
        }
        assert_eq!(
            expanded
                .iter()
                .filter_map(|token| match token {
                    Token::Char { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect::<String>(),
            "10.00037pt"
        );
        assert!(expanded.iter().all(|token| matches!(
            token,
            Token::Char {
                cat: Catcode::Other,
                ..
            }
        )));
    }

    #[test]
    fn pdf_ximage_bbox_rejects_missing_objects_and_bad_indices() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let missing = crate::run_memory_with_stores("\\message{\\pdfximagebbox99 1}", &mut stores)
            .expect_err("missing external image must be fatal");
        assert_eq!(
            missing.to_string(),
            "pdfTeX error (ext1): cannot find referenced object."
        );

        stores
            .register_pdf_external_image(
                tex_state::PdfExternalImageId::new(7).expect("PDF image id"),
                tex_state::PdfExternalImageMetadata::Raster,
            )
            .expect("register image metadata");
        for index in [0, 5, -1] {
            let error = crate::run_memory_with_stores(
                &format!("\\message{{\\pdfximagebbox7 {index}}}"),
                &mut stores,
            )
            .expect_err("bad bbox index must be fatal");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (pdfximagebbox): invalid parameter."
            );
        }
    }

    #[test]
    fn pdf_metadata_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_metadata_config", "ref");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/8/9",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let output = crate::run_memory_with_stores(
            include_str!("../../../tests/corpus/tex_exec/pdf_metadata_config.tex"),
            &mut stores,
        )
        .expect("pdfTeX metadata configuration assignments");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/8/9",
        ] {
            assert!(output.contains(expected), "missing {expected:?}: {output}");
        }
    }

    #[test]
    fn all_page_token_and_dimension_parameters_scan_group_and_display() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let mut source = String::new();
        for &(name, _, _) in PDFTEX_DIMEN_PARAMETERS {
            source.push_str(&format!("\\{name}=1pt "));
        }
        for &(name, _) in PDFTEX_TOK_PARAMETERS {
            source.push_str(&format!("\\{name}{{outer-{name}}} "));
        }
        source.push('{');
        for &(name, _, _) in PDFTEX_DIMEN_PARAMETERS {
            source.push_str(&format!("\\{name}=2pt \\message{{L{name}=\\the\\{name}}} "));
        }
        for &(name, _) in PDFTEX_TOK_PARAMETERS {
            source.push_str(&format!(
                "\\{name}{{inner-{name}}} \\message{{L{name}=\\the\\{name}}} "
            ));
        }
        source.push_str("} \\end");

        let output = crate::run_memory_with_stores(&source, &mut stores)
            .expect("all pdfTeX page parameters assign");
        for &(name, parameter, _) in PDFTEX_DIMEN_PARAMETERS {
            assert!(
                output.contains(&format!("L{name}=2.0pt")),
                "{name}: {output}"
            );
            assert_eq!(
                stores.dimen_param(parameter),
                Scaled::from_raw(Scaled::UNITY)
            );
        }
        for &(name, parameter) in PDFTEX_TOK_PARAMETERS {
            assert!(
                output.contains(&format!("L{name}=inner-{name}")),
                "{name}: {output}"
            );
            assert_eq!(
                token_list_text(&stores, stores.tok_param(parameter)),
                format!("outer-{name}")
            );
        }
    }

    #[test]
    fn pdftex_line_dimension_overrides_follow_ignore_and_precedence_rules() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        crate::run_memory_with_stores(
            concat!(
                "\\setbox0=\\vbox{\\hsize=10pt ",
                "\\pdfeachlineheight=11pt \\pdfeachlinedepth=12pt ",
                "\\pdffirstlineheight=13pt \\pdflastlinedepth=14pt ",
                "\\noindent\\hbox to10pt{}\\penalty-10000\\hbox to10pt{}\\par} ",
                "\\end",
            ),
            &mut stores,
        )
        .expect("pdfTeX line dimensions");

        let root = stores.box_reg(0).expect("setbox result");
        let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
            panic!("box0 is not a vbox");
        };
        let lines = stores
            .nodes(vbox.children)
            .into_iter()
            .filter_map(|node| match node {
                tex_state::node_arena::NodeRef::HList(line) => Some(line),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].height, Scaled::from_raw(13 * Scaled::UNITY));
        assert_eq!(lines[0].depth, Scaled::from_raw(12 * Scaled::UNITY));
        assert_eq!(lines[1].height, Scaled::from_raw(11 * Scaled::UNITY));
        assert_eq!(lines[1].depth, Scaled::from_raw(14 * Scaled::UNITY));
    }

    #[test]
    fn pdftex_parameters_survive_snapshots_hashes_and_formats() {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores.set_int_param(IntParam::PDF_COMPRESS_LEVEL, 5);
        stores.set_int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX, 2);
        stores.set_int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL, 3);
        stores.set_dimen_param(DimenParam::PDF_PAGE_WIDTH, Scaled::from_raw(12_345));
        let first_tokens = stores.intern_token_list(&[
            Token::Char {
                ch: 'f',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'i',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'r',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 's',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 't',
                cat: Catcode::Other,
            },
        ]);
        stores.set_tok_param(TokParam::PDF_PAGE_ATTR, first_tokens);
        let first = stores.snapshot();

        stores.set_int_param(IntParam::PDF_COMPRESS_LEVEL, 2);
        stores.set_int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX, 4);
        stores.set_int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL, 5);
        stores.set_dimen_param(DimenParam::PDF_PAGE_WIDTH, Scaled::from_raw(54_321));
        let second_tokens = stores.intern_token_list(&[Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        }]);
        stores.set_tok_param(TokParam::PDF_PAGE_ATTR, second_tokens);
        let second = stores.snapshot();
        assert_ne!(first.state_hash(), second.state_hash());

        stores.rollback(&first);
        let restored = stores.snapshot();
        assert_eq!(first.state_hash(), restored.state_hash());
        assert_eq!(stores.int_param(IntParam::PDF_COMPRESS_LEVEL), 5);
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            2
        );
        assert_eq!(
            stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
            3
        );
        assert_eq!(
            stores.dimen_param(DimenParam::PDF_PAGE_WIDTH),
            Scaled::from_raw(12_345)
        );
        assert_eq!(
            token_list_text(&stores, stores.tok_param(TokParam::PDF_PAGE_ATTR)),
            "first"
        );

        let format = stores.dump_format().expect("pdfTeX parameter format");
        let loaded = Universe::from_format(World::default(), &format).expect("load format");
        assert_eq!(loaded.dump_format().expect("redump format"), format);
        assert_eq!(loaded.int_param(IntParam::PDF_COMPRESS_LEVEL), 5);
        assert_eq!(
            loaded.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
            2
        );
        assert_eq!(
            loaded.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
            3
        );
        assert_eq!(
            loaded.dimen_param(DimenParam::PDF_PAGE_WIDTH),
            Scaled::from_raw(12_345)
        );
        assert_eq!(
            token_list_text(&loaded, loaded.tok_param(TokParam::PDF_PAGE_ATTR)),
            "first"
        );
    }

    fn token_list_text(stores: &Universe, id: TokenListId) -> String {
        stores
            .tokens(id)
            .iter()
            .filter_map(|token| match token {
                Token::Char { ch, .. } => Some(*ch),
                _ => None,
            })
            .collect()
    }
}
