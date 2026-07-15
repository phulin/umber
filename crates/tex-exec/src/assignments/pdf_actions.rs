//! Shared scanner for pdfTeX action specifications.

use tex_expand::scan::scan_general_text_expanded_with_driver;
use tex_lex::InputStack;
use tex_state::token::TracedTokenWord;
use tex_state::{
    PdfActionDestination, PdfActionIdentifier, PdfActionSpec, PdfActionTarget, PdfActionWindow,
    Universe,
};

use crate::{ExecError, ExecutionContext};

use super::{scan_i32, scan_optional_keyword_x};

pub(super) fn scan_pdf_action(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<PdfActionSpec, ExecError> {
    if scan_optional_keyword_x(input, stores, execution, "user")? {
        return Ok(PdfActionSpec::User(scan_text(
            context, input, stores, execution,
        )?));
    }
    let goto = if scan_optional_keyword_x(input, stores, execution, "goto")? {
        true
    } else if scan_optional_keyword_x(input, stores, execution, "thread")? {
        false
    } else {
        return Err(ExecError::PdfActionTypeMissing);
    };

    let file = if scan_optional_keyword_x(input, stores, execution, "file")? {
        Some(scan_text(context, input, stores, execution)?)
    } else {
        None
    };
    let structure = if scan_optional_keyword_x(input, stores, execution, "struct")? {
        if !goto {
            return Err(ExecError::PdfActionOnlyGoto("struct"));
        }
        if file.is_some() {
            Some(PdfActionIdentifier::Raw(scan_text(
                context, input, stores, execution,
            )?))
        } else {
            Some(scan_identifier(
                context, input, stores, execution, "struct",
            )?)
        }
    } else {
        None
    };

    let target = if scan_optional_keyword_x(input, stores, execution, "page")? {
        if !goto {
            return Err(ExecError::PdfActionOnlyGoto("page"));
        }
        let number = scan_positive(context, input, stores, execution, "page number")?;
        let view = scan_text(context, input, stores, execution)?;
        PdfActionTarget::Page { number, view }
    } else if scan_optional_keyword_x(input, stores, execution, "name")? {
        PdfActionTarget::Destination(PdfActionIdentifier::Name(scan_text(
            context, input, stores, execution,
        )?))
    } else if scan_optional_keyword_x(input, stores, execution, "num")? {
        if goto && file.is_some() {
            return Err(ExecError::PdfActionGotoFileNum);
        }
        PdfActionTarget::Destination(PdfActionIdentifier::Number(scan_positive(
            context,
            input,
            stores,
            execution,
            "num identifier",
        )?))
    } else {
        return Err(ExecError::PdfActionIdentifierTypeMissing);
    };

    let window = if scan_optional_keyword_x(input, stores, execution, "newwindow")? {
        PdfActionWindow::New
    } else if scan_optional_keyword_x(input, stores, execution, "nonewwindow")? {
        PdfActionWindow::Same
    } else {
        PdfActionWindow::Unspecified
    };
    if window != PdfActionWindow::Unspecified && (!goto || file.is_none()) {
        return Err(ExecError::PdfActionWindowRequiresGotoFile);
    }
    let destination = PdfActionDestination {
        file,
        structure,
        target,
        window,
    };
    Ok(if goto {
        PdfActionSpec::GoTo(destination)
    } else {
        PdfActionSpec::Thread(destination)
    })
}

fn scan_identifier(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    _kind: &'static str,
) -> Result<PdfActionIdentifier, ExecError> {
    if scan_optional_keyword_x(input, stores, execution, "name")? {
        Ok(PdfActionIdentifier::Name(scan_text(
            context, input, stores, execution,
        )?))
    } else if scan_optional_keyword_x(input, stores, execution, "num")? {
        Ok(PdfActionIdentifier::Number(scan_positive(
            context,
            input,
            stores,
            execution,
            "num identifier",
        )?))
    } else {
        Err(ExecError::PdfActionIdentifierTypeMissing)
    }
}

fn scan_positive(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    kind: &'static str,
) -> Result<u32, ExecError> {
    let value = scan_i32(input, stores, execution, context)?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or(ExecError::PdfActionPositiveIdentifier(kind))
}

fn scan_text(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<tex_state::ids::TokenListId, ExecError> {
    scan_general_text_expanded_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )
    .map_err(Into::into)
}
