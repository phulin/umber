use std::fmt;

use bumpalo::{Bump, collections::Vec as BumpVec};
use tex_out::PageNode;

use crate::{BatchResult, Workload, benchmark_font, build_artifact, character_node, kern_node};

mod input;

use input::{
    Control, Frame, PackedCursor, TAG_BEGIN_GROUP, TAG_CHAR, TAG_END_GROUP, TAG_PARAMETER, Token,
    lex_source,
};

#[derive(Debug)]
pub enum FusedError {
    UnexpectedEof(&'static str),
    UnexpectedToken(&'static str),
    UnknownControlSequence(String),
    UndefinedMacro,
    ArithmeticOverflow,
    Artifact(tex_out::ArtifactValidationError),
    Serialize(tex_out::SerializeError),
    Parse(tex_out::ParseError),
    Dvi(tex_out::dvi::DviError),
}

impl fmt::Display for FusedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fused batch failed: {self:?}")
    }
}

impl std::error::Error for FusedError {}

#[derive(Clone, Copy)]
struct SaveEntry {
    index: u8,
    old_value: i32,
    old_level: u16,
}

struct Kernel<'a> {
    bump: &'a Bump,
    frames: Vec<Frame<'a>>,
    backup: Option<Token>,
    macro_bodies: [Option<&'a [Token]>; 2],
    eqtb: [i32; 256],
    eq_levels: [u16; 256],
    save_entries: Vec<SaveEntry>,
    group_marks: Vec<usize>,
    nodes: Vec<PageNode>,
    global_prefix: bool,
    pending_shipout: bool,
    in_hbox: bool,
    calls: usize,
}

impl<'a> Kernel<'a> {
    fn new(bump: &'a Bump, source: &'a [u8], expected_calls: usize) -> Self {
        let mut frames = Vec::with_capacity(8);
        frames.push(Frame::Source {
            bytes: source,
            cursor: PackedCursor::source(),
        });
        Self {
            bump,
            frames,
            backup: None,
            macro_bodies: [None; 2],
            eqtb: [0; 256],
            eq_levels: [1; 256],
            save_entries: Vec::with_capacity(16),
            group_marks: Vec::with_capacity(4),
            nodes: Vec::with_capacity(expected_calls.saturating_mul(2)),
            global_prefix: false,
            pending_shipout: false,
            in_hbox: false,
            calls: 0,
        }
    }

    fn execute(mut self) -> Result<([i32; 3], Vec<PageNode>, usize), FusedError> {
        loop {
            let token = self
                .next_expanded()?
                .ok_or(FusedError::UnexpectedEof("explicit \\end"))?;
            if let Some(ch) = token.as_char() {
                if self.in_hbox && ch == crate::CHARACTER {
                    self.nodes.push(character_node());
                    self.calls += 1;
                    continue;
                }
                return Err(FusedError::UnexpectedToken("dispatch character"));
            }
            if token.tag() == TAG_END_GROUP {
                self.end_group()?;
                self.in_hbox = false;
                continue;
            }
            let control = token
                .as_control()
                .ok_or(FusedError::UnexpectedToken("dispatch control sequence"))?;
            match control {
                Control::Count => self.assign_count()?,
                Control::Def => self.define_macro()?,
                Control::Advance => self.advance_count()?,
                Control::Global => self.global_prefix = true,
                Control::IfNum => self.conditional()?,
                Control::Else => self.skip_to_fi()?,
                Control::Fi | Control::Relax => {}
                Control::Shipout => self.pending_shipout = true,
                Control::Hbox => self.begin_hbox()?,
                Control::Kern => self.emit_kern()?,
                Control::End => break,
                Control::EmitE | Control::EmitF => {
                    unreachable!("macro calls expand before dispatch")
                }
            }
        }
        if self.in_hbox || !self.group_marks.is_empty() {
            return Err(FusedError::UnexpectedEof("hbox group"));
        }
        Ok((
            [self.eqtb[0], self.eqtb[1], self.eqtb[2]],
            self.nodes,
            self.calls,
        ))
    }

    fn next_expanded(&mut self) -> Result<Option<Token>, FusedError> {
        loop {
            let Some(token) = self.next_raw()? else {
                return Ok(None);
            };
            if let Some(slot) = token.as_control().and_then(Control::macro_slot) {
                let body = self.macro_bodies[slot].ok_or(FusedError::UndefinedMacro)?;
                let argument = self.scan_macro_argument()?;
                self.frames.push(Frame::Packed {
                    tokens: body,
                    cursor: PackedCursor::tokens(),
                    argument: Some(argument),
                });
                continue;
            }
            return Ok(Some(token));
        }
    }

    fn next_raw(&mut self) -> Result<Option<Token>, FusedError> {
        if let Some(token) = self.backup.take() {
            return Ok(Some(token));
        }
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Ok(None);
            };
            match frame {
                Frame::Source { bytes, cursor } => {
                    debug_assert!(!cursor.is_token_cursor());
                    if let Some(token) = lex_source(bytes, cursor)? {
                        return Ok(Some(token));
                    }
                    self.frames.pop();
                }
                Frame::Packed {
                    tokens,
                    cursor,
                    argument,
                } => {
                    debug_assert!(cursor.is_token_cursor());
                    let Some(&token) = tokens.get(cursor.position()) else {
                        self.frames.pop();
                        continue;
                    };
                    cursor.advance();
                    if token.tag() == TAG_PARAMETER {
                        let argument = argument.ok_or(FusedError::UnexpectedToken(
                            "parameter outside macro replacement",
                        ))?;
                        self.frames.push(Frame::Packed {
                            tokens: argument,
                            cursor: PackedCursor::tokens(),
                            argument: None,
                        });
                        continue;
                    }
                    return Ok(Some(token));
                }
            }
        }
    }

    fn scan_macro_argument(&mut self) -> Result<&'a [Token], FusedError> {
        let opener = self
            .next_raw()?
            .ok_or(FusedError::UnexpectedEof("macro argument"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Ok(self.bump.alloc_slice_copy(&[opener]));
        }
        let mut depth = 1_usize;
        let mut tokens = BumpVec::new_in(self.bump);
        while depth != 0 {
            let token = self
                .next_raw()?
                .ok_or(FusedError::UnexpectedEof("braced macro argument"))?;
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

    fn define_macro(&mut self) -> Result<(), FusedError> {
        let target = self
            .next_raw()?
            .and_then(Token::as_control)
            .and_then(Control::macro_slot)
            .ok_or(FusedError::UnexpectedToken("macro definition target"))?;
        self.expect_char(b'#', "macro parameter marker")?;
        self.expect_char(b'1', "macro parameter number")?;
        let opener = self
            .next_raw()?
            .ok_or(FusedError::UnexpectedEof("macro replacement"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Err(FusedError::UnexpectedToken("macro replacement opener"));
        }
        let mut depth = 1_usize;
        let mut body = BumpVec::new_in(self.bump);
        while depth != 0 {
            let token = self
                .next_raw()?
                .ok_or(FusedError::UnexpectedEof("macro replacement"))?;
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
                        .ok_or(FusedError::UnexpectedToken("macro replacement parameter"))?;
                    if index != b'1' {
                        return Err(FusedError::UnexpectedToken("macro parameter index"));
                    }
                    body.push(Token::parameter(1));
                }
                _ => body.push(token),
            }
        }
        self.macro_bodies[target] = Some(body.into_bump_slice());
        Ok(())
    }

    fn assign_count(&mut self) -> Result<(), FusedError> {
        let index = self.scan_register_index()?;
        if self.peek_expanded_char()? == Some(b'=') {
            self.next_expanded()?;
        }
        let value = self.scan_number()?;
        self.write_count(index, value);
        Ok(())
    }

    fn advance_count(&mut self) -> Result<(), FusedError> {
        self.expect_expanded_control(Control::Count, "advance target")?;
        let index = self.scan_register_index()?;
        self.expect_expanded_char(b'b', "advance keyword")?;
        self.expect_expanded_char(b'y', "advance keyword")?;
        let amount = self.scan_number()?;
        let value = self.eqtb[usize::from(index)]
            .checked_add(amount)
            .ok_or(FusedError::ArithmeticOverflow)?;
        self.write_count(index, value);
        Ok(())
    }

    fn conditional(&mut self) -> Result<(), FusedError> {
        let left = self.scan_number()?;
        let relation = self
            .next_expanded()?
            .and_then(Token::as_char)
            .ok_or(FusedError::UnexpectedToken("ifnum relation"))?;
        let right = self.scan_number()?;
        let condition = match relation {
            b'<' => left < right,
            b'=' => left == right,
            b'>' => left > right,
            _ => return Err(FusedError::UnexpectedToken("ifnum relation")),
        };
        if !condition {
            self.skip_false_branch()?;
        }
        Ok(())
    }

    fn skip_false_branch(&mut self) -> Result<(), FusedError> {
        let mut depth = 0_usize;
        loop {
            let token = self
                .next_raw()?
                .ok_or(FusedError::UnexpectedEof("false conditional branch"))?;
            match token.as_control() {
                Some(Control::IfNum) => depth += 1,
                Some(Control::Fi) if depth == 0 => return Ok(()),
                Some(Control::Fi) => depth -= 1,
                Some(Control::Else) if depth == 0 => return Ok(()),
                _ => {}
            }
        }
    }

    fn skip_to_fi(&mut self) -> Result<(), FusedError> {
        let mut depth = 0_usize;
        loop {
            let token = self
                .next_raw()?
                .ok_or(FusedError::UnexpectedEof("true conditional branch"))?;
            match token.as_control() {
                Some(Control::IfNum) => depth += 1,
                Some(Control::Fi) if depth == 0 => return Ok(()),
                Some(Control::Fi) => depth -= 1,
                _ => {}
            }
        }
    }

    fn begin_hbox(&mut self) -> Result<(), FusedError> {
        if !self.pending_shipout {
            return Err(FusedError::UnexpectedToken("hbox outside shipout"));
        }
        let opener = self
            .next_expanded()?
            .ok_or(FusedError::UnexpectedEof("hbox opener"))?;
        if opener.tag() != TAG_BEGIN_GROUP {
            return Err(FusedError::UnexpectedToken("hbox opener"));
        }
        self.group_marks.push(self.save_entries.len());
        self.pending_shipout = false;
        self.in_hbox = true;
        Ok(())
    }

    fn end_group(&mut self) -> Result<(), FusedError> {
        let mark = self
            .group_marks
            .pop()
            .ok_or(FusedError::UnexpectedToken("unmatched group end"))?;
        for entry in self.save_entries.drain(mark..).rev() {
            let slot = usize::from(entry.index);
            if self.eq_levels[slot] != 1 {
                self.eqtb[slot] = entry.old_value;
                self.eq_levels[slot] = entry.old_level;
            }
        }
        Ok(())
    }

    fn emit_kern(&mut self) -> Result<(), FusedError> {
        if !self.in_hbox {
            return Err(FusedError::UnexpectedToken("kern outside hbox"));
        }
        let amount = self.scan_number()?;
        self.expect_expanded_char(b's', "scaled-point unit")?;
        self.expect_expanded_char(b'p', "scaled-point unit")?;
        self.nodes.push(kern_node(amount));
        Ok(())
    }

    fn scan_register_index(&mut self) -> Result<u8, FusedError> {
        let value = self.scan_number()?;
        u8::try_from(value).map_err(|_| FusedError::UnexpectedToken("count register index"))
    }

    fn scan_number(&mut self) -> Result<i32, FusedError> {
        let mut value = 0_i32;
        let mut saw_digit = false;
        loop {
            let token = self
                .next_expanded()?
                .ok_or(FusedError::UnexpectedEof("integer"))?;
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
                .ok_or(FusedError::ArithmeticOverflow)?;
        }
        saw_digit
            .then_some(value)
            .ok_or(FusedError::UnexpectedToken("integer"))
    }

    fn write_count(&mut self, index: u8, value: i32) {
        let slot = usize::from(index);
        let current_level = self.group_marks.len() as u16 + 1;
        if !self.global_prefix
            && !self.group_marks.is_empty()
            && self.eq_levels[slot] != current_level
        {
            self.save_entries.push(SaveEntry {
                index,
                old_value: self.eqtb[slot],
                old_level: self.eq_levels[slot],
            });
            self.eq_levels[slot] = current_level;
        }
        self.eqtb[slot] = value;
        if self.global_prefix {
            self.eq_levels[slot] = 1;
        }
        self.global_prefix = false;
    }

    fn peek_expanded_char(&mut self) -> Result<Option<u8>, FusedError> {
        let token = self.next_expanded()?;
        self.backup = token;
        Ok(token.and_then(Token::as_char))
    }

    fn expect_char(&mut self, expected: u8, context: &'static str) -> Result<(), FusedError> {
        let actual = self.next_raw()?.and_then(Token::as_char);
        (actual == Some(expected))
            .then_some(())
            .ok_or(FusedError::UnexpectedToken(context))
    }

    fn expect_expanded_char(
        &mut self,
        expected: u8,
        context: &'static str,
    ) -> Result<(), FusedError> {
        let actual = self.next_expanded()?.and_then(Token::as_char);
        (actual == Some(expected))
            .then_some(())
            .ok_or(FusedError::UnexpectedToken(context))
    }

    fn expect_expanded_control(
        &mut self,
        expected: Control,
        context: &'static str,
    ) -> Result<(), FusedError> {
        let actual = self.next_expanded()?.and_then(Token::as_control);
        (actual == Some(expected))
            .then_some(())
            .ok_or(FusedError::UnexpectedToken(context))
    }
}

pub fn run_fused(workload: &Workload) -> Result<BatchResult, FusedError> {
    let source = workload.source();
    let bump = Bump::new();
    let kernel = Kernel::new(&bump, &source, workload.calls());
    let (counts, nodes, calls) = kernel.execute()?;
    let font = benchmark_font();
    let artifact = build_artifact(&font, counts, nodes).map_err(FusedError::Artifact)?;
    let artifact_bytes = artifact.to_bytes().map_err(FusedError::Serialize)?;
    let artifact = tex_out::PageArtifact::from_bytes(&artifact_bytes).map_err(FusedError::Parse)?;
    let plan = tex_out::dvi::DviPagePlan::compile(&artifact).map_err(FusedError::Dvi)?;
    let dvi = crate::serialize_dvi(plan).map_err(FusedError::Dvi)?;
    let terminal = format!("[{}.{}.{}]", counts[0], counts[1], counts[2]).into_bytes();
    let log = terminal.clone();
    Ok(BatchResult {
        counts,
        artifact,
        artifact_bytes,
        dvi,
        effects: Vec::new(),
        terminal,
        log,
        calls,
        command_work: None,
    })
}
