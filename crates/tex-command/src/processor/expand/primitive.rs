//! Primitive expansion from its native opcode and invocation facts.

use crate::processor::expand_render::format_pdf_date;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use tex_state::interner::Symbol;
use tex_state::meaning::ExpandablePrimitive;
use tex_state::token::TracedTokenWord;

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(never)]
    pub(super) fn expand_primitive(
        &mut self,
        primitive: ExpandablePrimitive,
        spelling: TracedTokenWord,
        control_sequence: Option<Symbol>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        if let Some(kind) = crate::conditionals::ConditionalKind::from_primitive(primitive) {
            return self.with_expansion_depth(|processor| {
                if report_trace
                    && processor
                        .state
                        .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
                        > 1
                {
                    processor.trace_primitive(primitive, control_sequence);
                }
                processor.expand_conditional_occupied(primitive, kind)
            });
        }
        self.with_expansion_depth(|processor| {
            processor.expand_primitive_in(primitive, spelling, control_sequence, report_trace)
        })
    }

    #[cold]
    #[inline(never)]
    fn trace_primitive(
        &mut self,
        primitive: ExpandablePrimitive,
        control_sequence: Option<Symbol>,
    ) {
        self.print_command_trace(crate::PrintCommand::from_expandable(
            primitive,
            control_sequence,
        ));
    }

    #[inline(never)]
    fn expand_primitive_in(
        &mut self,
        primitive: ExpandablePrimitive,
        spelling: TracedTokenWord,
        control_sequence: Option<Symbol>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        #[cfg(feature = "profiling")]
        {
            tex_state::measurement::record_hot_core_expandable_opcode(
                usize::try_from(primitive.operand())
                    .expect("expandable primitive operand fits usize"),
            );
            if self.write_expansion_depth != 0 {
                self.record_write_expansion();
            }
        }
        // TeX82 §367 traces a primitive before its scanner consumes an
        // operand. `EndTemplate` is handled by the delivery loop's sentinel
        // branch and has no primitive trace of its own.
        if report_trace
            && primitive != ExpandablePrimitive::EndTemplate
            && (self
                .state
                .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
                > 1)
        {
            self.trace_primitive(primitive, control_sequence);
        }

        let result = match primitive {
            ExpandablePrimitive::Unless => self.expand_unless_compact(),
            primitive @ (ExpandablePrimitive::Else
            | ExpandablePrimitive::Or
            | ExpandablePrimitive::Fi) => {
                self.expand_conditional_delimiter(spelling, control_sequence, primitive)
            }
            _ => self.expand_primitive_nonconditional(primitive, spelling),
        };
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let error = result.expect_err("matched resource need");
            self.command
                .scratch
                .unwind_resource_failure()
                .map_err(crate::scan_toks::scratch_command_error)?;
            return Err(error);
        }
        result
    }

    /// Dispatches the non-conditional expandable families.  Conditional
    /// operands are the one deeply recursive ordinary path, so keeping this
    /// large closed match out of [`Self::expand_primitive`] avoids
    /// reserving its diagnostic and conversion temporaries in every waiting
    /// conditional frame.
    #[inline(never)]
    fn expand_primitive_nonconditional(
        &mut self,
        primitive: ExpandablePrimitive,
        spelling: TracedTokenWord,
    ) -> Result<(), CommandError> {
        let origin = spelling.origin();
        match primitive {
            ExpandablePrimitive::EndTemplate => self.insert_frozen_endv(),
            ExpandablePrimitive::NoExpand => self.expand_noexpand(),
            ExpandablePrimitive::ExpandAfter => self.expand_expandafter(),
            ExpandablePrimitive::CsName => self.expand_csname(origin),
            ExpandablePrimitive::String => self.expand_string(origin),
            ExpandablePrimitive::Meaning => self.expand_meaning(origin),
            ExpandablePrimitive::Number => self.expand_number(origin, false),
            ExpandablePrimitive::RomanNumeral => self.expand_number(origin, true),
            ExpandablePrimitive::The => {
                let scanned = self.scan_internal_value_or_zero()?;
                self.expand_the_value(origin, scanned.value)
            }
            ExpandablePrimitive::Unexpanded => self.expand_unexpanded(),
            ExpandablePrimitive::Expanded => self.expand_expanded(),
            ExpandablePrimitive::Detokenize => self.expand_detokenize(origin),
            ExpandablePrimitive::Scantokens => self.expand_scantokens(),
            ExpandablePrimitive::FontName => self.expand_fontname(origin),
            ExpandablePrimitive::PdfFontName => self.expand_pdf_font_name(origin),
            ExpandablePrimitive::PdfFontObjectNumber => self.expand_pdf_font_object_number(origin),
            ExpandablePrimitive::PdfFontSize => self.expand_pdf_font_size(origin),
            ExpandablePrimitive::LeftMarginKern | ExpandablePrimitive::RightMarginKern => {
                self.expand_margin_kern(origin, primitive)
            }
            ExpandablePrimitive::Input => self.expand_input(spelling),
            ExpandablePrimitive::EndInput => self.expand_endinput(),
            ExpandablePrimitive::JobName => {
                self.state.unsupported_host_capability();
                let job_name = self.host.job_name().to_owned();
                self.push_rendered_text(&job_name, origin);
                Ok(())
            }
            ExpandablePrimitive::ETeXRevision => {
                self.push_rendered_text(".6", origin);
                Ok(())
            }
            ExpandablePrimitive::PdfTeXRevision => {
                self.push_rendered_text("27", origin);
                Ok(())
            }
            ExpandablePrimitive::PdfTeXBanner => {
                self.push_rendered_text(
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                    origin,
                );
                Ok(())
            }
            ExpandablePrimitive::PdfUniformDeviate => self.expand_pdf_uniform_deviate(origin),
            ExpandablePrimitive::PdfNormalDeviate => {
                let value = self.state.pdf_normal_deviate();
                self.push_rendered_text(&value.to_string(), origin);
                Ok(())
            }
            ExpandablePrimitive::CreationDate => {
                let clock = self.state.job_clock();
                self.push_rendered_text(&format_pdf_date(clock, 0), origin);
                Ok(())
            }
            ExpandablePrimitive::ShellEscape => {
                let status = self
                    .state
                    .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                    .expect("the shell-escape status is an integer enquiry");
                self.push_rendered_text(&status.to_string(), origin);
                Ok(())
            }
            ExpandablePrimitive::StringCompare => self.expand_string_compare(origin),
            ExpandablePrimitive::PdfEscapeString => self.expand_pdf_escape_string(origin),
            ExpandablePrimitive::PdfEscapeHex => self.expand_pdf_escape_hex(origin),
            ExpandablePrimitive::PdfUnescapeHex => self.expand_pdf_unescape_hex(origin),
            ExpandablePrimitive::PdfColorStackInit => self.expand_pdf_color_stack_init(origin),
            ExpandablePrimitive::PdfMatch => self.expand_pdf_match(origin),
            ExpandablePrimitive::PdfLastMatch => self.expand_pdf_last_match(origin),
            ExpandablePrimitive::PdfFileDump => self.expand_pdf_file_dump(origin),
            ExpandablePrimitive::FileSize => self.expand_pdf_file_size(origin),
            ExpandablePrimitive::PdfFileModificationDate => {
                self.expand_pdf_file_modification_date(origin)
            }
            ExpandablePrimitive::PdfMdFiveSum => self.expand_pdf_md_five_sum(origin),
            ExpandablePrimitive::PdfInsertHeight => self.expand_pdf_insert_height(origin),
            ExpandablePrimitive::PdfXImageBBox => self.expand_pdf_ximage_bbox(origin),
            ExpandablePrimitive::PdfXFormName => self.expand_pdf_xform_name(origin),
            ExpandablePrimitive::PdfPageRef => self.expand_pdf_page_ref(origin),
            ExpandablePrimitive::PdfPrimitive => {
                let mut destination = None;
                match self.get_next_into(&mut destination)? {
                    DeliveryStatus::End => return Err(CommandError::input_invariant()),
                    DeliveryStatus::Command => {}
                    _ => unreachable!("ordinary raw delivery returns only commands"),
                }
                let target = destination
                    .take()
                    .expect("command status initializes destination");
                let Some(symbol) = target.control_sequence() else {
                    return Ok(());
                };
                let name = self.state.resolve(symbol);
                let Some(frozen) = self.state.primitive_token(name) else {
                    return Ok(());
                };
                self.back_input_token(TracedTokenWord::pack(frozen, target.origin()))
            }
            ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark => self.expand_mark(primitive),
            ExpandablePrimitive::TopMarks
            | ExpandablePrimitive::FirstMarks
            | ExpandablePrimitive::BotMarks
            | ExpandablePrimitive::SplitFirstMarks
            | ExpandablePrimitive::SplitBotMarks => self.expand_mark_class(primitive),
            ExpandablePrimitive::ETeXVersion
            | ExpandablePrimitive::IfPdfPrimitive
            | ExpandablePrimitive::PdfEscapeName => {
                Err(CommandError::UnsupportedExpandablePrimitive(primitive))
            }
            ExpandablePrimitive::EndCsName => Err(CommandError::input_invariant()),
            ExpandablePrimitive::Unless
            | ExpandablePrimitive::Else
            | ExpandablePrimitive::Or
            | ExpandablePrimitive::Fi => Err(CommandError::input_invariant()),
            _ => Err(CommandError::UnsupportedExpandablePrimitive(primitive)),
        }
    }
}
