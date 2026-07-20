use std::fmt::Write as _;

use md5::{Digest, Md5};
use tex_lex::InputStack;
use tex_state::token::TracedTokenWord;
use tex_state::{ExpansionState, InputOpenState};

use crate::{Dispatch, ExpandError, ExpansionContext, ExpansionMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PdfFileEnquiry {
    ModificationDate,
    MdFiveSum,
    Dump,
}

pub(crate) fn execute(
    enquiry: PdfFileEnquiry,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    match enquiry {
        PdfFileEnquiry::ModificationDate => {
            execute_modification_date(input, stores, expansion, mode, context)
        }
        PdfFileEnquiry::MdFiveSum => execute_md5(input, stores, expansion, mode, context),
        PdfFileEnquiry::Dump => execute_dump(input, stores, expansion, mode, context),
    }
}

pub(crate) fn creation_date(
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Dispatch {
    crate::pdf_strings::render_bytes(
        stores,
        format_pdf_date(expansion.job_clock, 0).as_bytes(),
        context,
    )
}

fn execute_modification_date(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let name = scan_filename(input, stores, expansion, mode, context)?;
    let Some(content) = resolve_file(stores, expansion, &name, context)? else {
        return Ok(Dispatch::Continue);
    };
    let Some(date) = content.modification_date() else {
        return Ok(Dispatch::Continue);
    };
    Ok(crate::pdf_strings::render_bytes(
        stores,
        format_pdf_date(date.clock, date.utc_offset_minutes).as_bytes(),
        context,
    ))
}

fn execute_md5(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let file = crate::scan_helpers::scan_optional_keyword_with_mode_and_context(
        input, stores, expansion, mode, "file",
    )?;
    let bytes = crate::pdf_strings::scan_expanded_bytes(input, stores, expansion, mode, context)?;
    let bytes = if file {
        let name = filename_from_bytes(&bytes);
        let Some(content) = resolve_file(stores, expansion, &name, context)? else {
            return Ok(Dispatch::Continue);
        };
        content.bytes().to_vec()
    } else {
        bytes
    };
    let digest = Md5::digest(bytes);
    let mut rendered = String::with_capacity(32);
    for byte in digest {
        let _ = write!(rendered, "{byte:02X}");
    }
    Ok(crate::pdf_strings::render_bytes(
        stores,
        rendered.as_bytes(),
        context,
    ))
}

fn execute_dump(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let offset = scan_nonnegative_option(
        "offset",
        "Bad file offset",
        input,
        stores,
        expansion,
        mode,
        context,
    )?;
    let length = scan_nonnegative_option(
        "length",
        "Bad dump length",
        input,
        stores,
        expansion,
        mode,
        context,
    )?;
    let name = scan_filename(input, stores, expansion, mode, context)?;
    let Some(content) = resolve_file(stores, expansion, &name, context)? else {
        return Ok(Dispatch::Continue);
    };
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(content.bytes().len());
    let end = start
        .saturating_add(usize::try_from(length).unwrap_or(usize::MAX))
        .min(content.bytes().len());
    let mut rendered = Vec::with_capacity(end.saturating_sub(start).saturating_mul(2));
    for &byte in &content.bytes()[start..end] {
        rendered.push(hex_digit(byte >> 4));
        rendered.push(hex_digit(byte & 0x0f));
    }
    Ok(crate::pdf_strings::render_bytes(stores, &rendered, context))
}

#[allow(clippy::too_many_arguments)]
fn scan_nonnegative_option(
    keyword: &str,
    negative_message: &str,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<i32, ExpandError> {
    if !crate::scan_helpers::scan_optional_keyword_with_mode_and_context(
        input, stores, expansion, mode, keyword,
    )? {
        return Ok(0);
    }
    let scanned =
        crate::scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        stores.report_expansion_diagnostic(&format!("\n! {diagnostic}.\n"));
    }
    if scanned.value() < 0 {
        stores.report_expansion_diagnostic(&format!(
            "\n! {negative_message} ({}).\n",
            scanned.value()
        ));
        Ok(0)
    } else {
        Ok(scanned.value())
    }
}

fn scan_filename(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<String, ExpandError> {
    crate::pdf_strings::scan_expanded_bytes(input, stores, expansion, mode, context)
        .map(|bytes| filename_from_bytes(&bytes))
}

fn filename_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

fn resolve_file(
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    name: &str,
    context: TracedTokenWord,
) -> Result<Option<tex_state::FileContent>, ExpandError> {
    let lookup = expansion
        .input_file_content(&mut stores.input_open_context(), name)
        .map_err(|message| ExpandError::InputOpen {
            name: name.to_owned(),
            message,
            context,
        })?;
    match lookup {
        crate::ResourceLookup::Available(content) => Ok(Some(content)),
        crate::ResourceLookup::Unavailable => Ok(None),
        crate::ResourceLookup::NeedResource(need) => Err(ExpandError::NeedResource(need)),
    }
}

fn format_pdf_date(clock: tex_state::JobClock, utc_offset_minutes: i16) -> String {
    let mut date = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}",
        clock.year,
        clock.month,
        clock.day,
        clock.time.div_euclid(60),
        clock.time.rem_euclid(60),
        clock.second,
    );
    if utc_offset_minutes == 0 {
        date.push('Z');
    } else {
        let sign = if utc_offset_minutes < 0 { '-' } else { '+' };
        let absolute = i32::from(utc_offset_minutes).abs();
        let _ = write!(date, "{sign}{:02}'{:02}'", absolute / 60, absolute % 60);
    }
    date
}

const fn hex_digit(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + nibble - 10
    }
}
