use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::GlueParam;
use tex_state::ids::{GlueId, TokenListId};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{GroupKind, Universe};

use crate::assignments::{has_catcode_meaning, next_non_space_x, scan_scaled};
use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::{ExecError, assignments};

pub(crate) fn scan_preamble<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<AlignState, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let kind = alignment_kind(primitive)?;
    let pack_spec = scan_pack_spec(input, stores, hooks, context)?;
    let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "alignment group",
    })?;
    if !has_catcode_meaning(stores, opener, Catcode::BeginGroup) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing { inserted while scanning alignment preamble.\n",
        );
        crate::push_tokens(input, stores, [opener]);
    }
    stores.enter_group_with_kind(GroupKind::Simple);

    let end_template = Token::frozen_end_template();
    let mut scanner = PreambleScanner::new(input, stores, hooks);
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

fn scan_pack_spec<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<AlignmentPackSpec, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if assignments::scan_optional_keyword_x(input, stores, hooks, "to")? {
        Ok(AlignmentPackSpec::Exactly(scan_scaled(
            input, stores, hooks, context,
        )?))
    } else if assignments::scan_optional_keyword_x(input, stores, hooks, "spread")? {
        Ok(AlignmentPackSpec::Spread(scan_scaled(
            input, stores, hooks, context,
        )?))
    } else {
        Ok(AlignmentPackSpec::Natural)
    }
}

fn scan_u_template<S, H>(
    scanner: &mut PreambleScanner<'_, S, H>,
    column: usize,
    loop_start: &mut Option<usize>,
) -> Result<TokenListId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut builder = scanner.stores.token_list_builder();
    let mut has_material = false;
    loop {
        let token = scanner.next_token()?.ok_or(ExecError::MissingToken {
            context: "alignment preamble",
        })?;
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
        if is_alignment_tab_token(token) && !has_material && loop_start.is_none() {
            *loop_start = Some(column);
            continue;
        }
        if is_alignment_tab_token(token) || is_cr_token(scanner.stores, token) {
            scanner.lookahead = Some(token);
            scanner.stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Missing # inserted in alignment preamble.\nThere should be exactly one # between &'s, when an\n\\halign or \\valign is being set up. In this case you had\nnone, so I've put one in; maybe that will work.\n",
            );
            return Ok(scanner.stores.finish_token_list(&mut builder));
        }
        builder.push(token);
        has_material = true;
    }
}

fn scan_v_template<S, H>(
    scanner: &mut PreambleScanner<'_, S, H>,
    end_template: Token,
) -> Result<(TokenListId, PreambleTerminator), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut builder = scanner.stores.token_list_builder();
    loop {
        let token = scanner.next_token()?.ok_or(ExecError::MissingToken {
            context: "alignment preamble",
        })?;
        if is_parameter_token(token) {
            return Err(ExecError::ExtraHashInAlignmentPreamble);
        }
        if is_alignment_tab_token(token) {
            builder.push(end_template);
            return Ok((
                scanner.stores.finish_token_list(&mut builder),
                PreambleTerminator::AlignmentTab,
            ));
        }
        if is_cr_token(scanner.stores, token) {
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

struct PreambleScanner<'a, S, H> {
    input: &'a mut InputStack<S>,
    stores: &'a mut Universe,
    hooks: &'a mut H,
    lookahead: Option<Token>,
    current_tabskip: GlueId,
}

impl<'a, S, H> PreambleScanner<'a, S, H>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    fn new(input: &'a mut InputStack<S>, stores: &'a mut Universe, hooks: &'a mut H) -> Self {
        let current_tabskip = stores.glue_param(GlueParam::TAB_SKIP);
        Self {
            current_tabskip,
            input,
            stores,
            hooks,
            lookahead: None,
        }
    }

    fn current_tabskip(&self) -> GlueId {
        self.current_tabskip
    }

    fn next_is_alignment_tab(&mut self) -> Result<bool, ExecError> {
        let Some(token) = self.next_token()? else {
            return Ok(false);
        };
        if is_alignment_tab_token(token) {
            Ok(true)
        } else {
            self.lookahead = Some(token);
            Ok(false)
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ExecError> {
        if let Some(token) = self.lookahead.take() {
            return Ok(Some(token));
        }
        loop {
            // TeX82's get_preamble_token copies ordinary tokens without
            // expansion. Template macros must observe the state of each cell
            // when they are replayed; only \span requests an expansion here.
            let Some(token) = self.next_raw()? else {
                return Ok(None);
            };
            if self.try_scan_tabskip_assignment(token)? {
                continue;
            }
            if is_span_token(self.stores, token) {
                let Some(token) = self.next_expanded()? else {
                    return Err(ExecError::MissingToken {
                        context: "token after \\span",
                    });
                };
                if self.try_scan_tabskip_assignment(token)? {
                    continue;
                }
                return Ok(Some(token));
            }
            return Ok(Some(token));
        }
    }

    fn next_raw(&mut self) -> Result<Option<Token>, ExecError> {
        self.input.next_token(self.stores).map_err(ExecError::from)
    }

    fn next_expanded(&mut self) -> Result<Option<Token>, ExecError> {
        let mut recorder = NoopRecorder;
        Ok(
            get_x_token_with_recorder_and_hooks(
                self.input,
                self.stores,
                &mut recorder,
                self.hooks,
            )?
            .map(tex_expand::semantic_token),
        )
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
            self.hooks,
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
