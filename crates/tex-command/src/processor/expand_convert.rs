//! TeX and e-TeX conversion and mark primitives.

use tex_state::meaning::ExpandablePrimitive;
use tex_state::token::{OriginId, Token};

use crate::input::{PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};
use crate::{CommandError, CurrentCommand};

use super::expand_render::{
    append_scaled_without_unit, format_scaled, meaning_text, page_mark, render_the_value,
    roman_numeral, string_text,
};
use super::{CommandProcessor, DeliveryStatus};

impl<G> CommandProcessor<'_, '_, G> {
    /// e-TeX 2.6 etex.ch §53a's `\detokenize`.
    ///
    /// `scan_general_text` collects without expansion, `token_show` renders
    /// the frozen spelling exactly as for `\scantokens`, and `str_toks`
    /// projects the resulting string to category-10 spaces and category-12
    /// other characters.
    pub(super) fn expand_detokenize(
        &mut self,
        opener: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// pdftex.web §§495 and 1535's `\expanded` conversion.
    ///
    /// `scan_pdf_ext_toks` is exactly `scan_toks(false, true)`: it expands one
    /// balanced general-text argument and returns the resulting token list via
    /// `ins_list`. The inserted list therefore reenters the caller's current
    /// expansion loop instead of being rendered to characters.
    pub(super) fn expand_expanded(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let replacement = scanned.replacement_text;
        let words = self
            .command
            .attempt
            .arena()
            .token_words(replacement)
            .map_err(crate::scan_toks::attempt_command_error)?
            .to_vec();
        let first = words.first().map(|word| word.semantic_token());
        self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
        Ok(())
    }

    /// `\\string` observes spelling, never an effective control-sequence meaning.
    pub(super) fn expand_string(&mut self, opener: &CurrentCommand<G>) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        self.push_rendered_text(
            &string_text(self.state, target.spelling().semantic_token()),
            opener.origin(),
        );
        Ok(())
    }

    pub(super) fn expand_meaning(
        &mut self,
        opener: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        let text = meaning_text(self.state, &target);
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    pub(super) fn expand_number(
        &mut self,
        opener: &CurrentCommand<G>,
        roman: bool,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::Number {
                roman: retained_roman,
            } if retained_roman == roman => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let value = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::Number { roman },
                suspended,
            )?
            .value;
        let text = if roman {
            roman_numeral(value)
        } else {
            value.to_string()
        };
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// Expands TeX82 `the_toks` after command-owned internal-quantity scanning.
    ///
    /// The internal scanner owns a primitive register's `scan_eight_bit_int`
    /// episode.  In particular, `\\the\\count21` must deliver both index digits
    /// before it backs up the next source token and installs rendered output.
    /// Reaching into the target meaning here would leave that index to a later
    /// scanner and changes the observable input ordering.
    pub(super) fn expand_the(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::The
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_internal_value_or_zero_retained();
        let target = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::The,
            suspended,
        )?;
        self.expand_the_value(opener.origin(), target.value)
    }

    /// Installs one TeX82 §467 `ins_the_toks` result.
    ///
    /// §465's `the_toks` produces a token list for every `cur_val_level`: the
    /// scalar levels through `@<Convert |cur_val| to a token list@>`, `ident_val`
    /// as the font's own control-sequence token, and `tok_val` as a copy of the
    /// register or parameter. §467 then hands _all_ of them to the same
    /// `ins_list`, so none of the three may install a differently classified
    /// input level.
    pub(crate) fn expand_the_value(
        &mut self,
        opener: OriginId,
        value: crate::InternalValue,
    ) -> Result<(), CommandError> {
        if let Some(text) = render_the_value(&value) {
            self.push_rendered_text(&text, opener);
        } else {
            match value {
                // §466 copies the register's list rather than sharing its
                // durable source. The operation-local copy remains in the
                // attempt until this inserted level has copied its words.
                crate::InternalValue::Tokens { tokens } => {
                    let words = self
                        .command
                        .attempt
                        .arena()
                        .token_words(tokens)
                        .map_err(crate::scan_toks::attempt_command_error)?
                        .to_vec();
                    let first = words.first().map(|word| word.semantic_token());
                    self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
                }
                crate::InternalValue::Font(symbol) => {
                    self.push_rendered_tokens([Token::Cs(symbol)], opener);
                }
                _ => unreachable!("non-token internal values are rendered above"),
            }
        }
        Ok(())
    }

    /// TeX82 §471's `font_name_code: scan_font_ident` and §472's
    /// `print(font_name[cur_val])`.
    ///
    /// `\fontname` owns no operand reading of its own: §577's
    /// `scan_font_ident` is the only routine that turns a command into a
    /// font, including its invalid-identifier recovery to `nullfont`.
    pub(super) fn expand_fontname(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::FontName
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::FontName,
            suspended,
        )?;
        let mut name = self.state.font_name(font);
        let size = self.state.font_size(font);
        if size != self.state.font_design_size(font) {
            // TeX82 §472 appends `at <size>pt` whenever the selected size
            // differs from the TFM design size. This text is inserted as
            // catcode-12/space tokens by `str_toks`, so it must be complete
            // before an enclosing `\edef` captures it.
            name.push_str(" at ");
            append_scaled_without_unit(size, &mut name);
            name.push_str("pt");
        }
        self.push_rendered_text(&name, opener.origin());
        Ok(())
    }

    pub(super) fn expand_pdf_font_size(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::PdfFontSize
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfFontSize,
            suspended,
        )?;
        let size = format_scaled(self.state.tracked_font_size(font));
        self.push_rendered_text(&size, opener.origin());
        Ok(())
    }

    pub(super) fn expand_margin_kern(
        &mut self,
        opener: CurrentCommand<G>,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::PdfMarginKern {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let index = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfMarginKern { primitive },
            suspended,
        )?;
        let side = match primitive {
            ExpandablePrimitive::LeftMarginKern => tex_state::node::MarginKernSide::Left,
            ExpandablePrimitive::RightMarginKern => tex_state::node::MarginKernSide::Right,
            _ => return Err(CommandError::input_invariant()),
        };
        let Some(amount) = self.state.box_margin_kern(index, side) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (marginkern): a non-empty hbox expected",
            ));
        };
        self.push_rendered_text(&format_scaled(amount), opener.origin());
        Ok(())
    }

    pub(super) fn expand_mark(
        &mut self,
        primitive: ExpandablePrimitive,
    ) -> Result<(), CommandError> {
        if let Some(tokens) = self.state.page_mark_value(page_mark(primitive)).cloned() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    pub(super) fn expand_mark_class(
        &mut self,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        // e-TeX 2.6 `etex.ch` [26.1178] uses the same
        // `scan_register_num` as numbered marks and sparse registers.
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::MarkClass {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let class = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::MarkClass { primitive },
            suspended,
        )?;
        // e-TeX 2.6 etex.ch [25.386] makes class zero an exact alias for
        // TeX82's `cur_mark`, including its null-versus-empty pointer state.
        let tokens = self
            .state
            .page_mark_class_value(page_mark(primitive), class);
        if let Some(tokens) = tokens.cloned() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    fn push_mark_text(&mut self, tokens: &tex_state::node::NodeTokenList) {
        let words = tokens.words();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::stored_semantic(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(crate::input::StoredReplayReason::Mark),
        );
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Mark,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
    }
}
