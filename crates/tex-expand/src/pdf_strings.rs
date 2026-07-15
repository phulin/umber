use tex_lex::InputStack;
use tex_state::ExpansionState;
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::values::push_rendered_tokens;
use crate::{
    Dispatch, ExpandError, ExpansionContext, ExpansionMode, ExpansionReplayKind,
    append_token_string_text,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PdfStringConversion {
    EscapeString,
    EscapeName,
    EscapeHex,
    UnescapeHex,
}

pub(crate) fn execute_conversion(
    conversion: PdfStringConversion,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let input_bytes = scan_expanded_bytes(input, stores, expansion, mode, context)?;
    let output = match conversion {
        PdfStringConversion::EscapeString => escape_string(&input_bytes),
        PdfStringConversion::EscapeName => escape_name(&input_bytes),
        PdfStringConversion::EscapeHex => escape_hex(&input_bytes),
        PdfStringConversion::UnescapeHex => unescape_hex(&input_bytes),
    };
    Ok(render_bytes(stores, &output, context))
}

pub(crate) fn execute_compare(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let left = scan_expanded_bytes(input, stores, expansion, mode, context)?;
    let right = scan_expanded_bytes(input, stores, expansion, mode, context)?;
    let result = match left.cmp(&right) {
        std::cmp::Ordering::Less => b"-1".as_slice(),
        std::cmp::Ordering::Equal => b"0".as_slice(),
        std::cmp::Ordering::Greater => b"1".as_slice(),
    };
    Ok(render_bytes(stores, result, context))
}

fn scan_expanded_bytes(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Vec<u8>, ExpandError> {
    let text = crate::scan::scan_general_text_expanded_with_expanded_open(
        input, stores, expansion, mode, context,
    )?;
    let mut rendered = String::new();
    for &token in stores.tokens(text.token_list()) {
        append_token_string_text(stores, token, &mut rendered);
    }
    let mut bytes = Vec::with_capacity(rendered.len());
    for ch in rendered.chars() {
        if u32::from(ch) <= u32::from(u8::MAX) {
            bytes.push(ch as u8);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    Ok(bytes)
}

fn render_bytes(
    stores: &mut tex_state::ExpansionContext<'_>,
    bytes: &[u8],
    context: TracedTokenWord,
) -> Dispatch {
    push_rendered_tokens(
        stores,
        ExpansionReplayKind::NumberOutput,
        bytes.iter().map(|&byte| Token::Char {
            ch: char::from(byte),
            cat: if byte == b' ' {
                Catcode::Space
            } else {
                Catcode::Other
            },
        }),
        context.origin(),
    )
}

fn escape_string(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for &byte in input {
        if !(b'!'..=b'~').contains(&byte) {
            output.push(b'\\');
            output.push(b'0' + ((byte >> 6) & 0x03));
            output.push(b'0' + ((byte >> 3) & 0x07));
            output.push(b'0' + (byte & 0x07));
        } else {
            if matches!(byte, b'(' | b')' | b'\\') {
                output.push(b'\\');
            }
            output.push(byte);
        }
    }
    output
}

fn escape_name(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for &byte in input {
        if byte == 0 {
            continue;
        }
        if byte <= 32
            || byte >= 127
            || matches!(
                byte,
                b'#' | b'%' | b'(' | b')' | b'/' | b'<' | b'>' | b'[' | b']' | b'{' | b'}'
            )
        {
            output.push(b'#');
            output.push(hex_digit(byte >> 4));
            output.push(hex_digit(byte & 0x0f));
        } else {
            output.push(byte);
        }
    }
    output
}

fn escape_hex(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() * 2);
    for &byte in input {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn unescape_hex(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() / 2 + 1);
    let mut high = None;
    for &byte in input {
        let Some(nibble) = hex_value(byte) else {
            continue;
        };
        if let Some(high) = high.take() {
            output.push(high | nibble);
        } else {
            high = Some(nibble << 4);
        }
    }
    if let Some(high) = high {
        output.push(high);
    }
    output
}

const fn hex_digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        10..=15 => b'A' + nibble - 10,
        _ => unreachable!(),
    }
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
