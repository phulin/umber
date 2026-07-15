//! pdfTeX font-map and per-font output actions.

use tex_expand::append_token_string_text;
use tex_expand::scan::scan_general_text_expanded_with_driver;
use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::token::TracedTokenWord;

use crate::ExecError;

use super::scan_font_selector;

pub(super) fn execute_pdf_font_output_action(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let font = match primitive {
        UnexpandablePrimitive::PdfFontAttr | UnexpandablePrimitive::PdfIncludeChars => {
            Some(scan_font_selector(input, stores, execution)?)
        }
        UnexpandablePrimitive::PdfMapFile | UnexpandablePrimitive::PdfMapLine => None,
        _ => unreachable!("caller restricts PDF font output actions"),
    };
    let tokens = scan_general_text_expanded_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    let bytes = token_list_bytes(stores, tokens);

    match primitive {
        UnexpandablePrimitive::PdfFontAttr => {
            stores.set_pdf_font_attribute(font.expect("font action scanned a font"), bytes);
        }
        UnexpandablePrimitive::PdfIncludeChars => {
            stores.include_pdf_font_chars(font.expect("font action scanned a font"), bytes);
        }
        UnexpandablePrimitive::PdfMapFile => {
            let file = tex_fonts::PdfFontMapFile::parse(&bytes)?;
            stores.push_pdf_font_map(tex_state::PdfFontMapOperation::File(file));
        }
        UnexpandablePrimitive::PdfMapLine => {
            let line = tex_fonts::PdfFontMapEntry::parse(&bytes)?;
            let duplicate_count = stores.pdf_font_map_duplicate_names().len();
            stores.push_pdf_font_map(tex_state::PdfFontMapOperation::Line(line));
            let duplicates = stores.pdf_font_map_duplicate_names();
            if duplicates.len() > duplicate_count
                && stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_MAP) <= 0
            {
                let name = String::from_utf8_lossy(
                    duplicates
                        .last()
                        .expect("a newly recorded duplicate has a name"),
                );
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!(
                        "\npdfTeX warning: pdftex: fontmap entry for `{name}' already exists, duplicates ignored\n"
                    ),
                );
            }
        }
        _ => unreachable!("caller restricts PDF font output actions"),
    }
    Ok(())
}

fn token_list_bytes(stores: &Universe, tokens: tex_state::ids::TokenListId) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(tokens) {
        append_token_string_text(stores, token, &mut text);
    }
    text.into_bytes()
}
