use tex_expand::expand_once_then_get_token_with_context;
use tex_lex::InputStack;
use tex_state::env::banks::GlueParam;
use tex_state::ids::{GlueId, TokenListId};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{ExpansionState, GroupKind, Universe};

use crate::assignments::{has_catcode_meaning, next_non_space_x, scan_scaled};
use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::{ExecError, assignments};

pub(crate) fn scan_preamble(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<AlignState, ExecError> {
    let kind = alignment_kind(primitive)?;
    let pack_spec = scan_pack_spec(input, stores, execution, context)?;
    let opener = loop {
        let opener =
            next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
                context: "alignment group",
            })?;
        let relax = match opener {
            Token::Cs(symbol) => stores.meaning(symbol) == Meaning::Relax,
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = stores.intern_active_character(ch);
                stores.meaning(symbol) == Meaning::Relax
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => false,
        };
        if !relax {
            break opener;
        }
    };
    if !has_catcode_meaning(stores, opener, Catcode::BeginGroup) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing { inserted while scanning alignment preamble.\n",
        );
        crate::push_tokens(input, stores, [opener]);
    }
    stores.enter_group_with_kind(GroupKind::Simple);
    // TeX82 resets the global scanner sentinel after scan_spec has consumed
    // the alignment opener and before copying the preamble.
    input.set_alignment_state(-1_000_000);

    let end_template = stores.frozen_end_template_token();
    let mut scanner = PreambleScanner::new(input, stores, execution);
    let mut columns = Vec::new();
    let mut tabskips = vec![scanner.current_tabskip()];
    let mut loop_start = None;

    loop {
        let boundary = columns.len();
        ensure_boundary(&mut tabskips, boundary, scanner.current_tabskip());

        let u_template = scan_u_template(&mut scanner, columns.len(), &mut loop_start)?;
        let (v_template, terminator) = scan_v_template(&mut scanner, end_template)?;
        columns.push(AlignColumn {
            u_template,
            v_template,
        });
        ensure_boundary(&mut tabskips, columns.len(), scanner.current_tabskip());

        match terminator {
            PreambleTerminator::Cr => break,
            PreambleTerminator::AlignmentTab => {
                if scanner.next_is_alignment_tab()? {
                    loop_start.get_or_insert(columns.len());
                }
            }
        }
    }

    Ok(AlignState::new(
        kind,
        pack_spec,
        columns,
        tabskips,
        scanner.current_tabskip(),
        loop_start,
    ))
}

fn alignment_kind(primitive: UnexpandablePrimitive) -> Result<AlignmentKind, ExecError> {
    match primitive {
        UnexpandablePrimitive::HAlign => Ok(AlignmentKind::HAlign),
        UnexpandablePrimitive::VAlign => Ok(AlignmentKind::VAlign),
        _ => unreachable!("caller restricts alignment primitives"),
    }
}

fn scan_pack_spec(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<AlignmentPackSpec, ExecError> {
    if assignments::scan_optional_keyword_x(input, stores, execution, "to")? {
        Ok(AlignmentPackSpec::Exactly(scan_scaled(
            input, stores, execution, context,
        )?))
    } else if assignments::scan_optional_keyword_x(input, stores, execution, "spread")? {
        Ok(AlignmentPackSpec::Spread(scan_scaled(
            input, stores, execution, context,
        )?))
    } else {
        Ok(AlignmentPackSpec::Natural)
    }
}

fn scan_u_template(
    scanner: &mut PreambleScanner<'_, '_>,
    column: usize,
    loop_start: &mut Option<usize>,
) -> Result<TokenListId, ExecError> {
    let mut builder = scanner.stores.token_list_builder();
    let mut has_material = false;
    loop {
        let token = match scanner.next_token()? {
            Some(PreambleToken::Token(token)) => token,
            Some(PreambleToken::RecoveryCr) => {
                scanner.lookahead = Some(PreambleToken::RecoveryCr);
                scanner.report_missing_parameter();
                return Ok(scanner.stores.finish_token_list(&mut builder));
            }
            None => unreachable!("preamble EOF is converted to recovery tokens"),
        };
        if is_parameter_token(token) {
            return Ok(scanner.stores.finish_token_list(&mut builder));
        }
        // TeX82 removes spacer commands from the start of every u-template.
        // Otherwise source formatting after `&` becomes material at the
        // beginning of every cell and incorrectly enlarges column maxima.
        if !has_material && assignments::has_catcode_meaning(scanner.stores, token, Catcode::Space)
        {
            continue;
        }
        // TeX82 records a leading tab on an empty u-template as `cur_loop`;
        // it is a periodic-preamble marker, not a missing-parameter error.
        if scanner.at_template_level()
            && is_alignment_tab_token(token)
            && !has_material
            && loop_start.is_none()
        {
            *loop_start = Some(column);
            continue;
        }
        if scanner.at_template_level()
            && (is_alignment_tab_token(token) || is_cr_token(scanner.stores, token))
        {
            scanner.lookahead = Some(PreambleToken::Token(token));
            scanner.report_missing_parameter();
            return Ok(scanner.stores.finish_token_list(&mut builder));
        }
        builder.push(token);
        has_material = true;
    }
}

fn scan_v_template(
    scanner: &mut PreambleScanner<'_, '_>,
    end_template: Token,
) -> Result<(TokenListId, PreambleTerminator), ExecError> {
    let mut builder = scanner.stores.token_list_builder();
    loop {
        let token = match scanner.next_token()? {
            Some(PreambleToken::Token(token)) => token,
            Some(PreambleToken::RecoveryCr) => {
                builder.push(end_template);
                return Ok((
                    scanner.stores.finish_token_list(&mut builder),
                    PreambleTerminator::Cr,
                ));
            }
            None => unreachable!("preamble EOF is converted to recovery tokens"),
        };
        if is_parameter_token(token) {
            scanner.stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Only one # is allowed per tab.\nThere should be exactly one # between &'s, when an\n\\halign or \\valign is being set up. In this case you had\nmore than one, so I'm ignoring all but the first.\n",
            );
            continue;
        }
        if scanner.at_template_level() && is_alignment_tab_token(token) {
            builder.push(end_template);
            return Ok((
                scanner.stores.finish_token_list(&mut builder),
                PreambleTerminator::AlignmentTab,
            ));
        }
        if scanner.at_template_level() && is_cr_token(scanner.stores, token) {
            builder.push(end_template);
            return Ok((
                scanner.stores.finish_token_list(&mut builder),
                PreambleTerminator::Cr,
            ));
        }
        builder.push(token);
    }
}

fn ensure_boundary(tabskips: &mut Vec<GlueId>, boundary: usize, current: GlueId) {
    while tabskips.len() <= boundary {
        tabskips.push(current);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreambleTerminator {
    AlignmentTab,
    Cr,
}

struct PreambleScanner<'a, 'ctx> {
    input: &'a mut InputStack,
    stores: &'a mut Universe,
    execution: &'a mut crate::ExecutionContext<'ctx>,
    lookahead: Option<PreambleToken>,
    current_tabskip: GlueId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreambleToken {
    Token(Token),
    /// TeX82's inaccessible `frozen_cr`, used only by scanner recovery.
    RecoveryCr,
}

impl<'a, 'ctx> PreambleScanner<'a, 'ctx> {
    fn new(
        input: &'a mut InputStack,
        stores: &'a mut Universe,
        execution: &'a mut crate::ExecutionContext<'ctx>,
    ) -> Self {
        let current_tabskip = stores.glue_param(GlueParam::TAB_SKIP);
        Self {
            current_tabskip,
            input,
            stores,
            execution,
            lookahead: None,
        }
    }

    fn current_tabskip(&self) -> GlueId {
        self.current_tabskip
    }

    fn at_template_level(&self) -> bool {
        self.input.alignment_state_is(-1_000_000)
    }

    fn next_is_alignment_tab(&mut self) -> Result<bool, ExecError> {
        let Some(token) = self.next_token()? else {
            return Ok(false);
        };
        if matches!(token, PreambleToken::Token(token) if self.at_template_level() && is_alignment_tab_token(token))
        {
            Ok(true)
        } else {
            self.lookahead = Some(token);
            Ok(false)
        }
    }

    fn next_token(&mut self) -> Result<Option<PreambleToken>, ExecError> {
        if let Some(token) = self.lookahead.take() {
            return Ok(Some(token));
        }
        loop {
            // TeX82's get_preamble_token copies ordinary tokens without
            // expansion. Template macros must observe the state of each cell
            // when they are replayed; only \span requests an expansion here.
            let Some(read) = self.next_raw()? else {
                return Ok(Some(self.recover_preamble_eof()));
            };
            let PreambleToken::Token(mut token) = read else {
                return Ok(Some(read));
            };
            while is_span_token(self.stores, token) {
                let Some(expanded) = self.next_expanded()? else {
                    return Ok(Some(self.recover_preamble_eof()));
                };
                let PreambleToken::Token(expanded) = expanded else {
                    return Ok(Some(expanded));
                };
                token = expanded;
            }
            if self.try_scan_tabskip_assignment(token)? {
                continue;
            }
            return Ok(Some(PreambleToken::Token(token)));
        }
    }

    fn next_raw(&mut self) -> Result<Option<PreambleToken>, ExecError> {
        let Some(traced) = tex_expand::next_semantic_raw_token(
            self.input,
            &mut tex_state::ExpansionContext::new(self.stores),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(self.recover_outer_or_token(traced)))
    }

    fn next_expanded(&mut self) -> Result<Option<PreambleToken>, ExecError> {
        let Some(traced) = expand_once_then_get_token_with_context(
            self.input,
            &mut tex_state::ExpansionContext::new(self.stores),
            self.execution,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(self.recover_outer_or_token(traced)))
    }

    fn recover_outer_or_token(&mut self, traced: TracedTokenWord) -> PreambleToken {
        let token = tex_expand::semantic_token(traced);
        if !is_outer_macro(self.stores, token) {
            return PreambleToken::Token(token);
        }

        // TeX.web §336 backs up the forbidden outer token, substitutes a
        // spacer for the current read, and inserts frozen \cr plus `}`. The
        // private RecoveryCr marker preserves frozen identity without making
        // an inaccessible engine token representable in user token lists.
        let right_brace = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let right_origin = self.stores.inserted_origin(
            InsertedOriginKind::ErrorRecovery,
            right_brace,
            traced.origin(),
        );
        self.input.back_input_alignment_token(traced);
        crate::insert_traced_tokens(
            self.input,
            self.stores,
            [TracedTokenWord::pack(right_brace, right_origin), traced],
        );
        self.lookahead = Some(PreambleToken::RecoveryCr);
        self.report_runaway_preamble();
        PreambleToken::Token(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        })
    }

    fn recover_preamble_eof(&mut self) -> PreambleToken {
        let right_brace = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let origin = self.stores.inserted_origin(
            InsertedOriginKind::ErrorRecovery,
            right_brace,
            tex_state::token::OriginId::UNKNOWN,
        );
        crate::insert_traced_tokens(
            self.input,
            self.stores,
            [TracedTokenWord::pack(right_brace, origin)],
        );
        self.report_runaway_preamble();
        PreambleToken::RecoveryCr
    }

    fn report_runaway_preamble(&mut self) {
        self.stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! File ended or forbidden control sequence found while scanning alignment preamble.\nI've inserted \\cr and a closing brace and will continue.\n",
        );
    }

    fn report_missing_parameter(&mut self) {
        self.stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing # inserted in alignment preamble.\nThere should be exactly one # between &'s, when an\n\\halign or \\valign is being set up. In this case you had\nnone, so I've put one in; maybe that will work.\n",
        );
    }

    fn try_scan_tabskip_assignment(&mut self, token: Token) -> Result<bool, ExecError> {
        if !is_tabskip_token(self.stores, token) {
            return Ok(false);
        }
        assignments::execute_assignment_meaning(
            Meaning::GlueParam(GlueParam::TAB_SKIP.raw()),
            tex_state::token::TracedTokenWord::pack(token, tex_state::token::OriginId::UNKNOWN),
            self.input,
            self.stores,
            self.execution,
        )?;
        self.current_tabskip = self.stores.glue_param(GlueParam::TAB_SKIP);
        Ok(true)
    }
}

fn is_parameter_token(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Parameter,
            ..
        }
    )
}

fn is_alignment_tab_token(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    )
}

fn is_tabskip_token(stores: &Universe, token: Token) -> bool {
    let Token::Cs(symbol) = token else {
        return false;
    };
    matches!(
        stores.meaning(symbol),
        Meaning::GlueParam(index) if index == GlueParam::TAB_SKIP.raw()
    )
}

fn is_outer_macro(stores: &Universe, token: Token) -> bool {
    let meaning = match token {
        Token::Cs(symbol) => stores.meaning(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores
            .active_character_symbol(ch)
            .map_or(Meaning::Undefined, |symbol| stores.meaning(symbol)),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => Meaning::Undefined,
    };
    matches!(
        meaning,
        Meaning::Macro { flags, .. } if flags.contains(tex_state::meaning::MeaningFlags::OUTER)
    )
}

fn is_span_token(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::Span)
}

fn is_cr_token(stores: &Universe, token: Token) -> bool {
    matches!(
        primitive_token(stores, token),
        Some(UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr)
    )
}

fn primitive_token(stores: &Universe, token: Token) -> Option<UnexpandablePrimitive> {
    let Token::Cs(symbol) = token else {
        return None;
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive) => Some(primitive),
        _ => None,
    }
}
