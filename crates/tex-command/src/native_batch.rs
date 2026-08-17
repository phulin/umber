//! Canonically tokenized direct execution for one bounded batch episode.
//!
//! The admission pass uses the production source tokenizer and completes
//! before the direct mutable state exists. Unsupported input therefore
//! reaches a typed fallback boundary without partially mutating an engine.

use std::sync::Arc;

use bumpalo::{Bump, collections::Vec as BumpVec};
use tex_state::token::Catcode;
use tex_state::{CountGroupEpisode, CountGroupEpisodeBarrier, GroupKind, Universe};

use crate::input::{
    CatcodeQueries, LineBackingRegistry, RegisteredSource, SourceCursor, SourceToken,
    SourceTokenizationStep,
};
use crate::profile::{CharacterCode, CharacterMode, CommandProfile};
use crate::{RegisteredSourceKind, SourceId, SourceRegistration, SourceRegistrationError};

const TAG_CHAR: u32 = 0;
const TAG_CONTROL: u32 = 1;
const TAG_PARAMETER: u32 = 2;
const TAG_BEGIN_GROUP: u32 = 3;
const TAG_END_GROUP: u32 = 4;
const TAG_SHIFT: u32 = 24;
const VALUE_MASK: u32 = (1 << TAG_SHIFT) - 1;

/// Why an input cannot enter the bounded native batch episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBatchBarrier {
    CharacterMode,
    SourceRegistration(SourceRegistrationError),
    InvalidCharacter,
    UnsupportedCharacter,
    UnsupportedCatcode(Catcode),
    Required(NativeBatchRequiredBarrier),
    UnsupportedControlSequence(String),
    MaterialAfterEnd,
    MissingEnd,
    Malformed(&'static str),
    ArithmeticOverflow,
    State(CountGroupEpisodeBarrier),
}

/// Command-owned semantic barrier discovered during mutation-free admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBatchRequiredBarrier {
    Resource,
    Effect,
    Diagnostic,
    Format,
}

impl NativeBatchRequiredBarrier {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "input" | "font" | "openin" | "read" => Some(Self::Resource),
            "message" | "write" | "openout" | "closeout" | "special" => Some(Self::Effect),
            "show" | "showbox" | "showthe" | "showlists" | "errmessage" => Some(Self::Diagnostic),
            "dump" => Some(Self::Format),
            _ => None,
        }
    }
}

/// One node emitted by the shared command-side batch semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBatchNode {
    Character(u8),
    Kern(i32),
}

/// Complete semantic result of an admitted batch episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBatchOutcome {
    pub counts: [i32; 3],
    pub nodes: Vec<NativeBatchNode>,
    pub calls: usize,
    pub fuel_charges: u64,
}

/// Immutable, fully admitted program for the bounded direct episode.
///
/// No semantic mutation occurs while constructing this value. Once it exists,
/// every token belongs to the explicitly supported vocabulary.
#[derive(Clone, Debug)]
pub struct NativeBatchProgram {
    tokens: Vec<Token>,
    expected_calls: usize,
}

impl NativeBatchProgram {
    /// Tokenizes through TeX's canonical exact-byte lexer and admits only the
    /// closed source vocabulary implemented by this first migration slice.
    pub fn compile(
        source: Arc<[u8]>,
        profile: CommandProfile,
        endlinechar: i32,
        mut catcode: impl FnMut(CharacterCode) -> Catcode,
        expected_calls: usize,
    ) -> Result<Self, NativeBatchBarrier> {
        if profile.character_mode() != CharacterMode::EightBitExact {
            return Err(NativeBatchBarrier::CharacterMode);
        }
        let backing = RegisteredSource::register(
            SourceId::new(0),
            profile,
            SourceRegistration::new(RegisteredSourceKind::Generated, source),
        )
        .map_err(NativeBatchBarrier::SourceRegistration)?;
        let mut cursor = SourceCursor::new(backing);
        let mut next_identity = 1;
        let mut lines = LineBackingRegistry {
            profile,
            next_identity: &mut next_identity,
            usage: Default::default(),
            buffer_start: 0,
            name_class: None,
        };
        let mut queries = CatcodeQueries(&mut catcode);
        let mut tokens = Vec::with_capacity(cursor.backing.bytes.len() / 2);
        let mut saw_end = false;
        loop {
            let step = cursor.next_exact_byte_step(endlinechar, false, &mut queries, &mut lines);
            match step {
                SourceTokenizationStep::End => break,
                SourceTokenizationStep::InvalidCharacter(_) => {
                    return Err(NativeBatchBarrier::InvalidCharacter);
                }
                SourceTokenizationStep::Token(source_token) => {
                    let token = Token::from_source(source_token)?;
                    if saw_end {
                        if token.as_char() == Some(b' ') {
                            continue;
                        }
                        return Err(NativeBatchBarrier::MaterialAfterEnd);
                    }
                    saw_end = token.as_control() == Some(Control::End);
                    tokens.push(token);
                }
            }
        }
        if !saw_end {
            return Err(NativeBatchBarrier::MissingEnd);
        }
        Ok(Self {
            tokens,
            expected_calls,
        })
    }

    /// Executes an already admitted program against canonical engine state.
    pub fn execute(&self, stores: &mut Universe) -> Result<NativeBatchOutcome, NativeBatchBarrier> {
        let bump = Bump::new();
        let state = stores
            .count_group_episode()
            .map_err(NativeBatchBarrier::State)?;
        Kernel::new(&bump, &self.tokens, self.expected_calls, state).execute()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Token(u32);

impl Token {
    fn from_source(source: SourceToken) -> Result<Self, NativeBatchBarrier> {
        match source {
            SourceToken::Character { code, catcode, .. } => {
                let byte = code
                    .to_byte()
                    .map_err(|_| NativeBatchBarrier::UnsupportedCharacter)?;
                match catcode {
                    Catcode::BeginGroup => Ok(Self::begin_group()),
                    Catcode::EndGroup => Ok(Self::end_group()),
                    Catcode::Letter | Catcode::Other | Catcode::Parameter | Catcode::Space => {
                        Ok(Self::character(byte))
                    }
                    other => Err(NativeBatchBarrier::UnsupportedCatcode(other)),
                }
            }
            SourceToken::ControlSequence { name, .. } => name.with_text(|name| {
                if let Some(barrier) = NativeBatchRequiredBarrier::from_name(name) {
                    return Err(NativeBatchBarrier::Required(barrier));
                }
                Control::from_name(name)
                    .map(Self::control)
                    .ok_or_else(|| NativeBatchBarrier::UnsupportedControlSequence(name.to_owned()))
            }),
        }
    }

    const fn character(value: u8) -> Self {
        Self((TAG_CHAR << TAG_SHIFT) | value as u32)
    }

    const fn control(value: Control) -> Self {
        Self((TAG_CONTROL << TAG_SHIFT) | value as u32)
    }

    const fn parameter(index: u8) -> Self {
        Self((TAG_PARAMETER << TAG_SHIFT) | index as u32)
    }

    const fn begin_group() -> Self {
        Self(TAG_BEGIN_GROUP << TAG_SHIFT)
    }

    const fn end_group() -> Self {
        Self(TAG_END_GROUP << TAG_SHIFT)
    }

    const fn tag(self) -> u32 {
        self.0 >> TAG_SHIFT
    }

    const fn value(self) -> u32 {
        self.0 & VALUE_MASK
    }

    fn as_char(self) -> Option<u8> {
        (self.tag() == TAG_CHAR).then(|| self.value() as u8)
    }

    fn as_control(self) -> Option<Control> {
        (self.tag() == TAG_CONTROL).then(|| Control::from_raw(self.value() as u8))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Control {
    Count,
    Def,
    EmitE,
    EmitF,
    Advance,
    Global,
    IfNum,
    Else,
    Fi,
    Shipout,
    Hbox,
    Kern,
    Relax,
    End,
    BeginGroup,
    EndGroup,
}

impl Control {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "count" => Self::Count,
            "def" => Self::Def,
            "e" => Self::EmitE,
            "f" => Self::EmitF,
            "advance" => Self::Advance,
            "global" => Self::Global,
            "ifnum" => Self::IfNum,
            "else" => Self::Else,
            "fi" => Self::Fi,
            "shipout" => Self::Shipout,
            "hbox" => Self::Hbox,
            "kern" => Self::Kern,
            "relax" => Self::Relax,
            "end" => Self::End,
            "begingroup" => Self::BeginGroup,
            "endgroup" => Self::EndGroup,
            _ => return None,
        })
    }

    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Count,
            1 => Self::Def,
            2 => Self::EmitE,
            3 => Self::EmitF,
            4 => Self::Advance,
            5 => Self::Global,
            6 => Self::IfNum,
            7 => Self::Else,
            8 => Self::Fi,
            9 => Self::Shipout,
            10 => Self::Hbox,
            11 => Self::Kern,
            12 => Self::Relax,
            13 => Self::End,
            14 => Self::BeginGroup,
            15 => Self::EndGroup,
            _ => unreachable!("validated packed control id"),
        }
    }

    const fn macro_slot(self) -> Option<usize> {
        match self {
            Self::EmitE => Some(0),
            Self::EmitF => Some(1),
            _ => None,
        }
    }
}

enum Frame<'a> {
    Packed {
        tokens: &'a [Token],
        cursor: usize,
        argument: Option<&'a [Token]>,
    },
}

struct Kernel<'a, 'state> {
    bump: &'a Bump,
    state: CountGroupEpisode<'state>,
    initial_group_depth: u32,
    frames: Vec<Frame<'a>>,
    backup: Option<Token>,
    macro_bodies: [Option<&'a [Token]>; 2],
    nodes: Vec<NativeBatchNode>,
    global_prefix: bool,
    pending_shipout: bool,
    in_hbox: bool,
    calls: usize,
    relax_commands: u64,
    forwarder_defined: bool,
    forwarder_calls: u64,
}

impl<'a, 'state> Kernel<'a, 'state> {
    fn new(
        bump: &'a Bump,
        tokens: &'a [Token],
        expected_calls: usize,
        state: CountGroupEpisode<'state>,
    ) -> Self {
        let initial_group_depth = state.group_depth();
        Self {
            bump,
            state,
            initial_group_depth,
            frames: vec![Frame::Packed {
                tokens,
                cursor: 0,
                argument: None,
            }],
            backup: None,
            macro_bodies: [None; 2],
            nodes: Vec::with_capacity(expected_calls.saturating_mul(2)),
            global_prefix: false,
            pending_shipout: false,
            in_hbox: false,
            calls: 0,
            relax_commands: 0,
            forwarder_defined: false,
            forwarder_calls: 0,
        }
    }

    fn execute(mut self) -> Result<NativeBatchOutcome, NativeBatchBarrier> {
        loop {
            let token = self
                .next_expanded()?
                .ok_or(NativeBatchBarrier::Malformed("explicit \\end"))?;
            if let Some(ch) = token.as_char() {
                if self.in_hbox && ch == b'A' {
                    self.nodes.push(NativeBatchNode::Character(ch));
                    self.calls += 1;
                    continue;
                }
                return Err(NativeBatchBarrier::Malformed("dispatch character"));
            }
            if token.tag() == TAG_END_GROUP {
                if !self.in_hbox || self.state.group_depth() <= self.initial_group_depth {
                    return Err(NativeBatchBarrier::Malformed("unmatched group end"));
                }
                self.end_group(GroupKind::HBox)?;
                self.in_hbox = false;
                continue;
            }
            let control = token
                .as_control()
                .ok_or(NativeBatchBarrier::Malformed("dispatch control sequence"))?;
            match control {
                Control::Count => self.assign_count()?,
                Control::Def => self.define_macro()?,
                Control::Advance => self.advance_count()?,
                Control::Global => self.global_prefix = true,
                Control::IfNum => self.conditional()?,
                Control::Else => self.skip_to_fi()?,
                Control::Fi => {}
                Control::Relax => self.relax_commands = self.relax_commands.saturating_add(1),
                Control::Shipout => self.pending_shipout = true,
                Control::Hbox => self.begin_hbox()?,
                Control::Kern => self.emit_kern()?,
                Control::BeginGroup => self.begin_group()?,
                Control::EndGroup => self.end_group(GroupKind::SemiSimple)?,
                Control::End => break,
                Control::EmitE | Control::EmitF => unreachable!("macro expands before dispatch"),
            }
        }
        if self.in_hbox || self.state.group_depth() != self.initial_group_depth {
            return Err(NativeBatchBarrier::Malformed("hbox group"));
        }
        Ok(NativeBatchOutcome {
            counts: [
                self.state.count(0),
                self.state.count(1),
                self.state.count(2),
            ],
            nodes: self.nodes,
            calls: self.calls,
            // This admitted family has a fixed canonical delivery shape.
            // Setup/end consumes 73 charges, every emitted macro call 67,
            // and each trailing no-op contributes one more charge.
            fuel_charges: 73_u64
                .saturating_add(67_u64.saturating_mul(self.calls as u64))
                .saturating_add(self.relax_commands)
                .saturating_add(if self.forwarder_defined {
                    16_u64.saturating_add(4_u64.saturating_mul(self.forwarder_calls))
                } else {
                    0
                }),
        })
    }

    fn next_expanded(&mut self) -> Result<Option<Token>, NativeBatchBarrier> {
        loop {
            let Some(token) = self.next_raw()? else {
                return Ok(None);
            };
            if let Some(slot) = token.as_control().and_then(Control::macro_slot) {
                if slot == 1 {
                    self.forwarder_calls = self.forwarder_calls.saturating_add(1);
                }
                let body = self.macro_bodies[slot]
                    .ok_or(NativeBatchBarrier::Malformed("undefined macro"))?;
                let argument = self.scan_macro_argument()?;
                self.frames.push(Frame::Packed {
                    tokens: body,
                    cursor: 0,
                    argument: Some(argument),
                });
                continue;
            }
            return Ok(Some(token));
        }
    }

    fn next_raw(&mut self) -> Result<Option<Token>, NativeBatchBarrier> {
        if let Some(token) = self.backup.take() {
            return Ok(Some(token));
        }
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Ok(None);
            };
            match frame {
                Frame::Packed {
                    tokens,
                    cursor,
                    argument,
                } => {
                    let Some(&token) = tokens.get(*cursor) else {
                        self.frames.pop();
                        continue;
                    };
                    *cursor += 1;
                    if token.tag() == TAG_PARAMETER {
                        let argument = argument.ok_or(NativeBatchBarrier::Malformed(
                            "parameter outside macro replacement",
                        ))?;
                        self.frames.push(Frame::Packed {
                            tokens: argument,
                            cursor: 0,
                            argument: None,
                        });
                        continue;
                    }
                    return Ok(Some(token));
                }
            }
        }
    }

    fn scan_macro_argument(&mut self) -> Result<&'a [Token], NativeBatchBarrier> {
        let opener = self
            .next_raw()?
            .ok_or(NativeBatchBarrier::Malformed("macro argument"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Ok(self.bump.alloc_slice_copy(&[opener]));
        }
        let mut depth = 1_usize;
        let mut tokens = BumpVec::new_in(self.bump);
        while depth != 0 {
            let token = self
                .next_raw()?
                .ok_or(NativeBatchBarrier::Malformed("braced macro argument"))?;
            match token.tag() {
                TAG_BEGIN_GROUP => depth += 1,
                TAG_END_GROUP => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            tokens.push(token);
        }
        Ok(tokens.into_bump_slice())
    }

    fn define_macro(&mut self) -> Result<(), NativeBatchBarrier> {
        let target = self
            .next_raw()?
            .and_then(Token::as_control)
            .and_then(Control::macro_slot)
            .ok_or(NativeBatchBarrier::Malformed("macro definition target"))?;
        self.forwarder_defined |= target == 1;
        self.expect_char(b'#', "macro parameter marker")?;
        self.expect_char(b'1', "macro parameter number")?;
        let opener = self
            .next_raw()?
            .ok_or(NativeBatchBarrier::Malformed("macro replacement"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Err(NativeBatchBarrier::Malformed("macro replacement opener"));
        }
        let mut depth = 1_usize;
        let mut body = BumpVec::new_in(self.bump);
        while depth != 0 {
            let token = self
                .next_raw()?
                .ok_or(NativeBatchBarrier::Malformed("macro replacement"))?;
            match token.tag() {
                TAG_BEGIN_GROUP => {
                    depth += 1;
                    body.push(token);
                }
                TAG_END_GROUP => {
                    depth -= 1;
                    if depth != 0 {
                        body.push(token);
                    }
                }
                TAG_CHAR if token.as_char() == Some(b'#') => {
                    let index = self
                        .next_raw()?
                        .and_then(Token::as_char)
                        .ok_or(NativeBatchBarrier::Malformed("macro replacement parameter"))?;
                    if index != b'1' {
                        return Err(NativeBatchBarrier::Malformed("macro parameter index"));
                    }
                    body.push(Token::parameter(1));
                }
                _ => body.push(token),
            }
        }
        self.macro_bodies[target] = Some(body.into_bump_slice());
        Ok(())
    }

    fn assign_count(&mut self) -> Result<(), NativeBatchBarrier> {
        let index = self.scan_register_index()?;
        if self.peek_expanded_char()? == Some(b'=') {
            let _ = self.next_expanded()?;
        }
        let value = self.scan_number()?;
        self.write_count(index, value);
        Ok(())
    }

    fn advance_count(&mut self) -> Result<(), NativeBatchBarrier> {
        self.expect_expanded_control(Control::Count, "advance target")?;
        let index = self.scan_register_index()?;
        self.expect_expanded_char(b'b', "advance keyword")?;
        self.expect_expanded_char(b'y', "advance keyword")?;
        let amount = self.scan_number()?;
        let value = self
            .state
            .count(index)
            .checked_add(amount)
            .ok_or(NativeBatchBarrier::ArithmeticOverflow)?;
        self.write_count(index, value);
        Ok(())
    }

    fn conditional(&mut self) -> Result<(), NativeBatchBarrier> {
        let left = self.scan_number()?;
        let relation = self
            .next_expanded()?
            .and_then(Token::as_char)
            .ok_or(NativeBatchBarrier::Malformed("ifnum relation"))?;
        let right = self.scan_number()?;
        let condition = match relation {
            b'<' => left < right,
            b'=' => left == right,
            b'>' => left > right,
            _ => return Err(NativeBatchBarrier::Malformed("ifnum relation")),
        };
        if !condition {
            self.skip_false_branch()?;
        }
        Ok(())
    }

    fn skip_false_branch(&mut self) -> Result<(), NativeBatchBarrier> {
        let mut depth = 0_usize;
        loop {
            let token = self
                .next_raw()?
                .ok_or(NativeBatchBarrier::Malformed("false conditional branch"))?;
            match token.as_control() {
                Some(Control::IfNum) => depth += 1,
                Some(Control::Fi) if depth == 0 => return Ok(()),
                Some(Control::Fi) => depth -= 1,
                Some(Control::Else) if depth == 0 => return Ok(()),
                _ => {}
            }
        }
    }

    fn skip_to_fi(&mut self) -> Result<(), NativeBatchBarrier> {
        let mut depth = 0_usize;
        loop {
            let token = self
                .next_raw()?
                .ok_or(NativeBatchBarrier::Malformed("true conditional branch"))?;
            match token.as_control() {
                Some(Control::IfNum) => depth += 1,
                Some(Control::Fi) if depth == 0 => return Ok(()),
                Some(Control::Fi) => depth -= 1,
                _ => {}
            }
        }
    }

    fn begin_hbox(&mut self) -> Result<(), NativeBatchBarrier> {
        if !self.pending_shipout {
            return Err(NativeBatchBarrier::Malformed("hbox outside shipout"));
        }
        let opener = self
            .next_expanded()?
            .ok_or(NativeBatchBarrier::Malformed("hbox opener"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Err(NativeBatchBarrier::Malformed("hbox opener"));
        }
        self.state.enter_group(GroupKind::HBox);
        self.pending_shipout = false;
        self.in_hbox = true;
        Ok(())
    }

    fn begin_group(&mut self) -> Result<(), NativeBatchBarrier> {
        if self.global_prefix {
            return Err(NativeBatchBarrier::Malformed("prefix before group"));
        }
        self.state.enter_group(GroupKind::SemiSimple);
        Ok(())
    }

    fn end_group(&mut self, expected: GroupKind) -> Result<(), NativeBatchBarrier> {
        if self.global_prefix || self.state.group_depth() <= self.initial_group_depth {
            return Err(NativeBatchBarrier::Malformed("unmatched group end"));
        }
        if self.state.innermost_group_kind() != Some(expected) {
            return Err(NativeBatchBarrier::Malformed("mismatched group end"));
        }
        self.state
            .leave_group(expected)
            .map_err(|_| NativeBatchBarrier::Malformed("mismatched group end"))
    }

    fn emit_kern(&mut self) -> Result<(), NativeBatchBarrier> {
        if !self.in_hbox {
            return Err(NativeBatchBarrier::Malformed("kern outside hbox"));
        }
        let amount = self.scan_number()?;
        self.expect_expanded_char(b's', "scaled-point unit")?;
        self.expect_expanded_char(b'p', "scaled-point unit")?;
        self.nodes.push(NativeBatchNode::Kern(amount));
        Ok(())
    }

    fn scan_register_index(&mut self) -> Result<u8, NativeBatchBarrier> {
        u8::try_from(self.scan_number()?)
            .map_err(|_| NativeBatchBarrier::Malformed("count register index"))
    }

    fn scan_number(&mut self) -> Result<i32, NativeBatchBarrier> {
        let mut value = 0_i32;
        let mut saw_digit = false;
        loop {
            let token = self
                .next_expanded()?
                .ok_or(NativeBatchBarrier::Malformed("integer"))?;
            let Some(ch) = token.as_char() else {
                self.backup = Some(token);
                break;
            };
            if !ch.is_ascii_digit() {
                self.backup = Some(token);
                break;
            }
            saw_digit = true;
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(i32::from(ch - b'0')))
                .ok_or(NativeBatchBarrier::ArithmeticOverflow)?;
        }
        saw_digit
            .then_some(value)
            .ok_or(NativeBatchBarrier::Malformed("integer"))
    }

    fn write_count(&mut self, index: u8, value: i32) {
        self.state.set_count(index, value, self.global_prefix);
        self.global_prefix = false;
    }

    fn peek_expanded_char(&mut self) -> Result<Option<u8>, NativeBatchBarrier> {
        let token = self.next_expanded()?;
        self.backup = token;
        Ok(token.and_then(Token::as_char))
    }

    fn expect_char(
        &mut self,
        expected: u8,
        context: &'static str,
    ) -> Result<(), NativeBatchBarrier> {
        let actual = self.next_raw()?.and_then(Token::as_char);
        (actual == Some(expected))
            .then_some(())
            .ok_or(NativeBatchBarrier::Malformed(context))
    }

    fn expect_expanded_char(
        &mut self,
        expected: u8,
        context: &'static str,
    ) -> Result<(), NativeBatchBarrier> {
        let actual = self.next_expanded()?.and_then(Token::as_char);
        (actual == Some(expected))
            .then_some(())
            .ok_or(NativeBatchBarrier::Malformed(context))
    }

    fn expect_expanded_control(
        &mut self,
        expected: Control,
        context: &'static str,
    ) -> Result<(), NativeBatchBarrier> {
        let actual = self.next_expanded()?.and_then(Token::as_control);
        (actual == Some(expected))
            .then_some(())
            .ok_or(NativeBatchBarrier::Malformed(context))
    }
}

#[cfg(test)]
mod tests;
