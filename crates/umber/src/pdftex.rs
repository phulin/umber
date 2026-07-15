//! Pinned pdfTeX 1.40.27 engine-layer inventory and mode registration.

use tex_state::Universe;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};

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

pub(crate) fn install_pdftex_layer(stores: &mut Universe) {
    for &name in PDFTEX_PRIMITIVE_NAMES {
        let symbol = stores.intern(name);
        stores.set_meaning(
            symbol,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfTeXUnimplemented),
        );
    }
    tex_expand::install_pdftex_expandable_primitives(stores);
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
    use tex_state::meaning::ExpandablePrimitive;
    use tex_state::token::Token;

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
}
