//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, MacroArguments, TokenListReplayKind};
use tex_state::interner::Symbol;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::stores::Stores;
use tex_state::token::Token;

pub mod args;
pub mod scan;
pub mod scan_dimen;
pub mod scan_int;

/// Records state reads performed by expansion.
///
/// The default implementation is `NoopRecorder`. Callers that need read sets
/// can supply a concrete recorder type and let monomorphization remove this
/// hook from ordinary builds.
pub trait ReadRecorder {
    fn record_meaning(&mut self, symbol: Symbol, meaning: Meaning);
}

/// Read recorder used when expansion tracing/incremental read sets are off.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopRecorder;

impl ReadRecorder for NoopRecorder {
    #[inline(always)]
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {}
}

/// Why `tex-expand` is replaying a frozen token list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionReplayKind {
    MacroBody,
    TheOutput,
    NumberOutput,
    Mark,
    Inserted,
}

impl ExpansionReplayKind {
    #[must_use]
    pub const fn as_lex_kind(self) -> TokenListReplayKind {
        match self {
            Self::MacroBody => TokenListReplayKind::MacroBody,
            Self::TheOutput | Self::NumberOutput => TokenListReplayKind::Inserted,
            Self::Mark => TokenListReplayKind::Mark,
            Self::Inserted => TokenListReplayKind::Inserted,
        }
    }
}

/// Expandable operation families owned by the gullet epic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandableOpcode {
    Macro,
    ExpandAfter,
    NoExpand,
    CsName,
    EndCsName,
    String,
    Number,
    RomanNumeral,
    Meaning,
    The,
    Input,
    If,
    Else,
    Or,
    Fi,
}

/// Result of one expansion dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Continue,
    Deliver(Token),
    DeliverNoExpand(Token),
    Push {
        replay_kind: ExpansionReplayKind,
        token_list: tex_state::ids::TokenListId,
        macro_arguments: MacroArguments,
    },
}

/// Errors raised by `get_x_token`.
#[derive(Debug)]
pub enum ExpandError {
    Lex(LexError),
    MacroCall(args::MacroCallError),
    UnimplementedExpandable(ExpandableOpcode),
    MissingTokenAfterPrimitive(ExpandableOpcode),
    MissingEndCsName,
    NonCharacterInCsName(Token),
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::MacroCall(err) => write!(f, "{err}"),
            Self::UnimplementedExpandable(opcode) => {
                write!(f, "expandable opcode {opcode:?} is not implemented yet")
            }
            Self::MissingTokenAfterPrimitive(opcode) => {
                write!(f, "missing token after expandable primitive {opcode:?}")
            }
            Self::MissingEndCsName => write!(f, "missing \\endcsname for \\csname"),
            Self::NonCharacterInCsName(token) => {
                write!(f, "non-character token {token:?} while scanning \\csname")
            }
        }
    }
}

impl std::error::Error for ExpandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
            Self::MacroCall(err) => Some(err),
            Self::UnimplementedExpandable(_)
            | Self::MissingTokenAfterPrimitive(_)
            | Self::MissingEndCsName
            | Self::NonCharacterInCsName(_) => None,
        }
    }
}

/// Narrow capability for `\csname`'s sanctioned state mutation.
pub trait CsNameInterner {
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol;
}

impl CsNameInterner for Stores {
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        Stores::intern_relaxed_control_sequence(self, name)
    }
}

impl From<LexError> for ExpandError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<args::MacroCallError> for ExpandError {
    fn from(value: args::MacroCallError) -> Self {
        Self::MacroCall(value)
    }
}

/// Pulls the next fully expanded token.
pub fn get_x_token<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
{
    get_x_token_with_recorder(input, stores, &mut NoopRecorder)
}

/// Pulls the next fully expanded token while recording meaning reads.
pub fn get_x_token_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    loop {
        let Some(read) = input.next_expansion_token_readonly(stores)? else {
            return Ok(None);
        };
        let token = read.token();

        if read.suppress_expansion() {
            return Ok(Some(token));
        }

        let Token::Cs(symbol) = token else {
            return Ok(Some(token));
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        match dispatch(token, input, stores, recorder, meaning)? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => return Ok(Some(token)),
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

/// Dispatches one token/meaning pair.
///
/// TODO(umber2-5qt.3): implement expandable primitive arms.
/// TODO(umber2-5qt.5): implement conditional scan/evaluation arms.
pub fn dispatch<S, R>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    match meaning {
        Meaning::Macro { flags, definition } if is_expandable_macro(flags) => {
            let macro_meaning = stores.macro_definition(definition);
            let arguments = args::match_macro_call_with_recorder(
                input,
                stores,
                recorder,
                token,
                macro_meaning,
            )?;
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::MacroBody,
                token_list: macro_meaning.replacement_text(),
                macro_arguments: arguments.as_macro_arguments(),
            })
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
            expand_after(input, stores, recorder)?;
            Ok(Dispatch::Continue)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
            let Some(token) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::NoExpand,
                ));
            };
            Ok(Dispatch::DeliverNoExpand(token))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
            let name = scan_csname(input, stores, recorder)?;
            let symbol = CsNameInterner::intern_relaxed_control_sequence(stores, &name);
            Ok(Dispatch::Deliver(Token::Cs(symbol)))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
            Ok(Dispatch::Deliver(token))
        }
        Meaning::Macro { .. }
        | Meaning::Undefined
        | Meaning::Relax
        | Meaning::CharGiven(_)
        | Meaning::Unknown(_) => Ok(Dispatch::Deliver(token)),
    }
}

const fn is_expandable_macro(flags: MeaningFlags) -> bool {
    !flags.contains(MeaningFlags::PROTECTED)
}

/// Skeleton dispatch table for all expandable opcode families in this epic.
pub fn dispatch_expandable_opcode(opcode: ExpandableOpcode) -> Result<(), ExpandError> {
    match opcode {
        ExpandableOpcode::Macro => Ok(()),
        ExpandableOpcode::ExpandAfter
        | ExpandableOpcode::NoExpand
        | ExpandableOpcode::CsName
        | ExpandableOpcode::EndCsName => Ok(()),
        ExpandableOpcode::String => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Number => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::RomanNumeral => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Meaning => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::The => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Input => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::If => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Else => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Or => Err(unimplemented_expandable(opcode)),
        ExpandableOpcode::Fi => Err(unimplemented_expandable(opcode)),
    }
}

fn unimplemented_expandable(opcode: ExpandableOpcode) -> ExpandError {
    ExpandError::UnimplementedExpandable(opcode)
}

fn expand_after<S, R>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let Some(saved) = input.next_token_readonly(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };
    let Some(target) = input.next_token_readonly(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };

    let target_dispatch = dispatch_one_raw_token(target, input, stores, recorder)?;
    push_dispatch_result(input, stores, target_dispatch);
    push_inserted_token(input, stores, saved);
    Ok(())
}

fn scan_csname<S, R>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let mut name = String::new();

    loop {
        let Some(read) = input.next_expansion_token_readonly(stores)? else {
            return Err(ExpandError::MissingEndCsName);
        };
        let token = read.token();

        if read.suppress_expansion() {
            append_csname_token(&mut name, token)?;
            continue;
        }

        let Token::Cs(symbol) = token else {
            append_csname_token(&mut name, token)?;
            continue;
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) {
            return Ok(name);
        }

        match dispatch(token, input, stores, recorder, meaning)? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                append_csname_token(&mut name, token)?;
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

fn append_csname_token(name: &mut String, token: Token) -> Result<(), ExpandError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExpandError::NonCharacterInCsName(token)),
    }
}

fn dispatch_one_raw_token<S, R>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let Token::Cs(symbol) = token else {
        return Ok(Dispatch::Deliver(token));
    };

    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    dispatch(token, input, stores, recorder, meaning)
}

fn push_dispatch_result<S>(input: &mut InputStack<S>, stores: &mut Stores, dispatch: Dispatch) {
    match dispatch {
        Dispatch::Deliver(token) => push_inserted_token(input, stores, token),
        Dispatch::DeliverNoExpand(token) => push_noexpand_token(input, stores, token),
        Dispatch::Continue => {}
        push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
    }
}

fn apply_dispatch_push<S>(input: &mut InputStack<S>, dispatch: Dispatch) {
    let Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments,
    } = dispatch
    else {
        return;
    };

    if replay_kind == ExpansionReplayKind::MacroBody {
        input.push_macro_body(token_list, macro_arguments);
    } else {
        input.push_token_list(token_list, replay_kind.as_lex_kind());
    }
}

fn push_inserted_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

fn push_noexpand_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::NoExpand);
}

#[cfg(test)]
mod tests {
    use super::{
        ExpandableOpcode, NoopRecorder, ReadRecorder, dispatch, dispatch_expandable_opcode,
        get_x_token, get_x_token_with_recorder,
    };
    use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
    use tex_state::interner::Symbol;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
    use tex_state::stores::Stores;
    use tex_state::token::{Catcode, Token};

    #[derive(Default)]
    struct CountingRecorder {
        reads: usize,
    }

    impl ReadRecorder for CountingRecorder {
        fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
            self.reads += 1;
        }
    }

    #[test]
    fn noop_recorder_has_no_state() {
        assert_eq!(core::mem::size_of::<NoopRecorder>(), 0);
    }

    #[test]
    fn dispatch_delivers_unexpandable_tokens() {
        let mut stores = Stores::new();
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        assert_eq!(
            dispatch(
                token,
                &mut InputStack::new(MemoryInput::new("")),
                &mut stores,
                &mut NoopRecorder,
                Meaning::Relax,
            )
            .expect("dispatch should succeed"),
            super::Dispatch::Deliver(token)
        );
    }

    #[test]
    fn expandable_dispatch_table_covers_epic_opcode_families() {
        let opcodes = [
            ExpandableOpcode::Macro,
            ExpandableOpcode::ExpandAfter,
            ExpandableOpcode::NoExpand,
            ExpandableOpcode::CsName,
            ExpandableOpcode::EndCsName,
            ExpandableOpcode::String,
            ExpandableOpcode::Number,
            ExpandableOpcode::RomanNumeral,
            ExpandableOpcode::Meaning,
            ExpandableOpcode::The,
            ExpandableOpcode::Input,
            ExpandableOpcode::If,
            ExpandableOpcode::Else,
            ExpandableOpcode::Or,
            ExpandableOpcode::Fi,
        ];

        for opcode in opcodes {
            let result = dispatch_expandable_opcode(opcode);
            match opcode {
                ExpandableOpcode::Macro
                | ExpandableOpcode::ExpandAfter
                | ExpandableOpcode::NoExpand
                | ExpandableOpcode::CsName
                | ExpandableOpcode::EndCsName => assert!(result.is_ok()),
                _ => assert!(matches!(
                    result,
                    Err(super::ExpandError::UnimplementedExpandable(found)) if found == opcode
                )),
            }
        }
    }

    #[test]
    fn get_x_token_delivers_unexpandable_control_sequence() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new(""));
        let list = stores.intern_token_list(&[Token::Cs(relax)]);
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(relax))
        );
    }

    #[test]
    fn get_x_token_pulls_from_source_frames_readonly() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new("x\\relax"));

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
            Some(Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            })
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
            Some(Token::Cs(relax))
        );
    }

    #[test]
    fn get_x_token_pushes_macro_body_frame_and_continues() {
        let mut stores = Stores::new();
        let macro_cs = stores.intern("m");
        let body = stores.intern_token_list(&[
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
        ]);
        let params = stores.intern_token_list(&[]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let invocation = stores.intern_token_list(&[Token::Cs(macro_cs)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            })
        );
        assert!(matches!(
            input.summary().frames().last(),
            Some(tex_lex::InputFrameSummary::TokenList {
                token_list,
                replay_kind: TokenListReplayKind::MacroBody,
                index: 1,
                macro_arguments
            }) if *token_list == body && macro_arguments.is_empty()
        ));
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            })
        );
    }

    #[test]
    fn recorder_observes_one_meaning_read_per_control_sequence_token() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let list = stores.intern_token_list(&[Token::Cs(relax)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);
        let mut recorder = CountingRecorder::default();

        assert_eq!(
            get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
                .expect("expansion should succeed"),
            Some(Token::Cs(relax))
        );
        assert_eq!(recorder.reads, 1);
    }

    #[test]
    fn expandafter_expands_second_token_then_replays_saved_token_first() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x'), char_token('y')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );

        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            char_token('a'),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "axy");
    }

    #[test]
    fn expandafter_chains_match_tex_pushback_order() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let first = stores.intern("first");
        let second = stores.intern("second");
        let params = stores.intern_token_list(&[]);
        let first_body = stores.intern_token_list(&[char_token('1')]);
        let second_body = stores.intern_token_list(&[char_token('2')]);
        stores.set_macro_meaning(
            first,
            MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
        );
        stores.set_macro_meaning(
            second,
            MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
        );

        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            Token::Cs(expandafter),
            Token::Cs(expandafter),
            Token::Cs(first),
            Token::Cs(second),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "12");
    }

    #[test]
    fn noexpand_suppresses_next_control_sequence_for_one_get_x_token() {
        let mut stores = Stores::new();
        let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(noexpand),
            Token::Cs(macro_cs),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(macro_cs))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('x'))
        );
    }

    #[test]
    fn expandafter_preserves_noexpand_for_later_frame_step() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            char_token('a'),
            Token::Cs(noexpand),
            Token::Cs(macro_cs),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('a'))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(macro_cs))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('x'))
        );
    }

    #[test]
    fn csname_interns_undefined_name_and_assigns_relax() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('f'),
            char_token('o'),
            char_token('o'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        let created = stores.symbol("foo");
        assert!(created.is_none());
        let token = get_x_token(&mut input, &mut stores)
            .expect("csname expansion should succeed")
            .expect("csname should emit a token");
        let Token::Cs(created) = token else {
            panic!("expected control sequence, got {token:?}");
        };

        assert_eq!(stores.resolve(created), "foo");
        assert_eq!(stores.meaning(created), Meaning::Relax);
    }

    #[test]
    fn csname_expands_name_pieces_before_interning() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let macro_cs = stores.intern("piece");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('b'), char_token('a'), char_token('r')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('f'),
            Token::Cs(macro_cs),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
            Some(Token::Cs(
                stores
                    .symbol("fbar")
                    .expect("expanded name should be interned")
            ))
        );
    }

    #[test]
    fn csname_reports_non_character_material_after_expansion() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let input_list =
            stores.intern_token_list(&[Token::Cs(csname), Token::Cs(relax), Token::Cs(endcsname)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert!(matches!(
            get_x_token(&mut input, &mut stores),
            Err(super::ExpandError::NonCharacterInCsName(Token::Cs(found))) if found == relax
        ));
    }

    #[test]
    fn csname_preserves_existing_meaning_for_ifx_relax_comparison() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let existing = stores.intern("known");
        stores.set_meaning(existing, Meaning::CharGiven('K'));
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('k'),
            char_token('n'),
            char_token('o'),
            char_token('w'),
            char_token('n'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
            Some(Token::Cs(existing))
        );
        assert_eq!(stores.meaning(existing), Meaning::CharGiven('K'));
    }

    #[test]
    fn csname_created_undefined_name_is_meaning_equal_to_relax() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('n'),
            char_token('e'),
            char_token('w'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        let Some(Token::Cs(created)) =
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed")
        else {
            panic!("expected created control sequence");
        };

        assert_eq!(stores.meaning(created), stores.meaning(relax));
    }

    #[test]
    fn macro_body_replay_substitutes_frozen_argument_lists() {
        let mut stores = Stores::new();
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let body = stores.intern_token_list(&[char_token('a'), Token::param(1), char_token('b')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let invocation = stores.intern_token_list(&[
            Token::Cs(macro_cs),
            char_token('{'),
            char_token('x'),
            char_token('y'),
            char_token('}'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "axyb");
    }

    #[test]
    fn nested_macro_calls_replay_arguments_from_outer_frozen_frame() {
        let mut stores = Stores::new();
        let wrap = stores.intern("wrap");
        let wrap_params = stores.intern_token_list(&[Token::param(1)]);
        let wrap_body =
            stores.intern_token_list(&[char_token('['), Token::param(1), char_token(']')]);
        stores.set_macro_meaning(
            wrap,
            MacroMeaning::new(MeaningFlags::EMPTY, wrap_params, wrap_body),
        );

        let outer = stores.intern("outer");
        let outer_params = stores.intern_token_list(&[Token::param(1)]);
        let outer_body = stores.intern_token_list(&[
            Token::Cs(wrap),
            char_token('{'),
            Token::param(1),
            char_token('}'),
        ]);
        stores.set_macro_meaning(
            outer,
            MacroMeaning::new(MeaningFlags::EMPTY, outer_params, outer_body),
        );

        let invocation = stores.intern_token_list(&[
            Token::Cs(outer),
            char_token('{'),
            char_token('x'),
            char_token('y'),
            char_token('}'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "[xy]");
    }

    #[test]
    fn identical_macro_bodies_keep_shared_body_identity_with_distinct_arguments() {
        let mut stores = Stores::new();
        let left = stores.intern("left");
        let right = stores.intern("right");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let first_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        let second_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        assert_eq!(first_body, second_body);
        stores.set_macro_meaning(
            left,
            MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
        );
        stores.set_macro_meaning(
            right,
            MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
        );

        let left_arg = stores.intern_token_list(&[char_token('x')]);
        let mut left_input = InputStack::new(MemoryInput::new(""));
        left_input.push_token_list(left_arg, TokenListReplayKind::Inserted);
        let left_meaning = stores.meaning(left);
        let left_dispatch = dispatch(
            Token::Cs(left),
            &mut left_input,
            &mut stores,
            &mut NoopRecorder,
            left_meaning,
        )
        .expect("left dispatch should succeed");
        let super::Dispatch::Push {
            token_list: left_body,
            macro_arguments: left_arguments,
            ..
        } = left_dispatch
        else {
            panic!("expected left macro body push");
        };
        assert_eq!(left_body, first_body);
        assert_eq!(
            stores.tokens(left_arguments.get(1).expect("left #1")),
            &[char_token('x')]
        );

        let right_arg = stores.intern_token_list(&[char_token('y')]);
        let mut right_input = InputStack::new(MemoryInput::new(""));
        right_input.push_token_list(right_arg, TokenListReplayKind::Inserted);
        let right_meaning = stores.meaning(right);
        let right_dispatch = dispatch(
            Token::Cs(right),
            &mut right_input,
            &mut stores,
            &mut NoopRecorder,
            right_meaning,
        )
        .expect("right dispatch should succeed");
        let super::Dispatch::Push {
            token_list: right_body,
            macro_arguments: right_arguments,
            ..
        } = right_dispatch
        else {
            panic!("expected right macro body push");
        };
        assert_eq!(right_body, second_body);
        assert_eq!(
            stores.tokens(right_arguments.get(1).expect("right #1")),
            &[char_token('y')]
        );

        let invocation = stores.intern_token_list(&[
            Token::Cs(left),
            char_token('x'),
            Token::Cs(right),
            char_token('y'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "x!y!");
    }

    fn next_expanded_chars(input: &mut InputStack<MemoryInput>, stores: &mut Stores) -> String {
        let mut out = String::new();
        while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
            let Token::Char { ch, .. } = token else {
                panic!("expected character token, got {token:?}");
            };
            out.push(ch);
        }
        out
    }

    fn char_token(ch: char) -> Token {
        let cat = match ch {
            '{' => Catcode::BeginGroup,
            '}' => Catcode::EndGroup,
            '[' | ']' | '!' => Catcode::Other,
            _ => Catcode::Letter,
        };
        Token::Char { ch, cat }
    }

    fn expandable_primitive(
        stores: &mut Stores,
        name: &str,
        primitive: ExpandablePrimitive,
    ) -> Symbol {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
        symbol
    }

    fn csname_primitives(stores: &mut Stores) -> (Symbol, Symbol) {
        (
            expandable_primitive(stores, "csname", ExpandablePrimitive::CsName),
            expandable_primitive(stores, "endcsname", ExpandablePrimitive::EndCsName),
        )
    }
}
