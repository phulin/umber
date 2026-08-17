//! Fused execution over the canonical command input stack.
//!
//! The episode owns no source bytes, token vector, tokenizer, cursor, or
//! input frame.  It repeatedly asks the production [`crate::CommandProcessor`]
//! for expanded commands, so physical input, registered sources, token-list
//! levels, backup, `\noexpand`, macro arguments, alignment templates, live
//! category codes, and root exhaustion all have exactly one owner.  A caller
//! wraps an attempt in the ordinary aggregate retry snapshot; an unsupported
//! semantic family can therefore resume scalar execution from the exact same
//! canonical input state.

use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::Catcode;
use tex_state::{CountGroupEpisode, CountGroupEpisodeBarrier, GroupKind};

use crate::{CommandError, CommandProcessor, CurrentCommand};

/// Why a canonical-input episode cannot absorb the next semantic action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBatchBarrier {
    Required(NativeBatchRequiredBarrier),
    UnsupportedCommand(Meaning),
    RootCompletion,
    Command(CommandError),
    Malformed(&'static str),
    ArithmeticOverflow,
    State(CountGroupEpisodeBarrier),
}

/// Command-owned semantic barrier reached by canonical delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBatchRequiredBarrier {
    Resource,
    Effect,
    Diagnostic,
    Format,
}

/// Borrowed node-construction capability supplied by the canonical executor.
///
/// The command core names semantic material but owns no node vector or node
/// store. The executor routes these calls into the same mutable builder used
/// by ordinary mode construction.
pub trait NativeBatchNodeSink {
    fn reserve(&mut self, additional: usize);
    fn character(&mut self, ch: u8);
    fn kern(&mut self, amount: i32);
}

/// Complete semantic result of one canonical-input episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBatchOutcome {
    pub counts: [i32; 3],
    pub calls: usize,
    pub fuel_charges: u64,
}

/// Immutable operational plan retained by production `MainControl`.
///
/// The plan contains only a capacity hint. All future semantic input remains
/// in `CommandState`; retaining this value cannot duplicate or stale a source
/// cursor.
#[derive(Clone, Debug)]
pub struct NativeBatchProgram {
    expected_calls: usize,
}

impl NativeBatchProgram {
    #[must_use]
    pub const fn new(expected_calls: usize) -> Self {
        Self { expected_calls }
    }

    /// Executes against the same canonical processor used by scalar command
    /// delivery. The processor owns all input and expansion transitions.
    pub fn execute<S: NativeBatchNodeSink>(
        &self,
        processor: &mut CommandProcessor<'_>,
        nodes: &mut S,
    ) -> Result<NativeBatchOutcome, NativeBatchBarrier> {
        let state = processor
            .begin_count_group_episode()
            .map_err(NativeBatchBarrier::State)?;
        Kernel::new(processor, self.expected_calls, state, nodes).execute()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Control {
    Count,
    Advance,
    Global,
    Shipout,
    Hbox,
    Kern,
    Relax,
    End,
    BeginGroup,
    EndGroup,
}

impl Control {
    fn from_command(command: &CurrentCommand) -> Option<Self> {
        let primitive = match command.meaning() {
            Meaning::Relax => return Some(Self::Relax),
            Meaning::UnexpandablePrimitive(primitive) => primitive,
            _ => return None,
        };
        Some(match primitive {
            UnexpandablePrimitive::Count => Self::Count,
            UnexpandablePrimitive::Advance => Self::Advance,
            UnexpandablePrimitive::Global => Self::Global,
            UnexpandablePrimitive::Shipout => Self::Shipout,
            UnexpandablePrimitive::HBox => Self::Hbox,
            UnexpandablePrimitive::Kern => Self::Kern,
            UnexpandablePrimitive::End => Self::End,
            UnexpandablePrimitive::BeginGroup => Self::BeginGroup,
            UnexpandablePrimitive::EndGroup => Self::EndGroup,
            _ => return None,
        })
    }
}

struct Kernel<'processor, 'state, 'nodes, S> {
    processor: &'processor mut CommandProcessor<'state>,
    state: Option<CountGroupEpisode>,
    initial_group_depth: u32,
    nodes: &'nodes mut S,
    expected_calls: usize,
    initial_effect_pos: tex_state::EffectPos,
    global_prefix: bool,
    pending_shipout: bool,
    in_hbox: bool,
    calls: usize,
}

impl<'processor, 'state, 'nodes, S: NativeBatchNodeSink> Kernel<'processor, 'state, 'nodes, S> {
    fn new(
        processor: &'processor mut CommandProcessor<'state>,
        expected_calls: usize,
        state: CountGroupEpisode,
        nodes: &'nodes mut S,
    ) -> Self {
        let initial_group_depth = processor.episode_group_depth(&state);
        let initial_effect_pos = processor.episode_effect_pos();
        Self {
            processor,
            state: Some(state),
            initial_group_depth,
            nodes,
            expected_calls,
            initial_effect_pos,
            global_prefix: false,
            pending_shipout: false,
            in_hbox: false,
            calls: 0,
        }
    }

    fn state(&self) -> &CountGroupEpisode {
        self.state.as_ref().expect("episode state remains live")
    }

    fn execute(mut self) -> Result<NativeBatchOutcome, NativeBatchBarrier> {
        loop {
            let command = self
                .next_expanded()?
                .ok_or(NativeBatchBarrier::RootCompletion)?;
            if let Some((ch, catcode)) = command_character(&command) {
                match catcode {
                    Catcode::Space if !self.in_hbox => continue,
                    Catcode::BeginGroup if !self.in_hbox => {
                        self.begin_group(GroupKind::Simple)?;
                        continue;
                    }
                    Catcode::EndGroup => {
                        let expected = if self.in_hbox {
                            GroupKind::HBox
                        } else {
                            GroupKind::Simple
                        };
                        self.end_group(expected)?;
                        self.in_hbox = false;
                        continue;
                    }
                    _ if self.in_hbox && ch == b'A' => {
                        self.nodes.character(ch);
                        self.calls += 1;
                        continue;
                    }
                    _ => return Err(NativeBatchBarrier::UnsupportedCommand(command.meaning())),
                }
            }
            let Some(control) = Control::from_command(&command) else {
                if let Some(required) = required_barrier(command.meaning()) {
                    return Err(NativeBatchBarrier::Required(required));
                }
                return Err(NativeBatchBarrier::UnsupportedCommand(command.meaning()));
            };
            match control {
                Control::Count => self.assign_count()?,
                Control::Advance => self.advance_count()?,
                Control::Global => self.global_prefix = true,
                Control::Relax => {}
                Control::Shipout => self.pending_shipout = true,
                Control::Hbox => self.begin_hbox()?,
                Control::Kern => self.emit_kern()?,
                Control::BeginGroup => self.begin_group(GroupKind::SemiSimple)?,
                Control::EndGroup => self.end_group(GroupKind::SemiSimple)?,
                Control::End => break,
            }
        }
        if self.in_hbox || self.group_depth() != self.initial_group_depth {
            return Err(NativeBatchBarrier::Malformed("hbox group"));
        }
        let counts = [self.count(0), self.count(1), self.count(2)];
        let fuel_charges = self
            .processor
            .episode_work()
            .fuel_charges
            // The canonical processor accounts raw and expanded delivery.
            // Its scalar integer, keyword, and dimension scanners now also
            // account the fourteen scan actions per emitted call which the
            // retired episode-local scanner used to leave to this receipt.
            // The retained executor slice owns only the character/kern pair,
            // plus the output/end pair after the loop.
            .saturating_add(2_u64.saturating_mul(self.calls as u64))
            .saturating_add(2);
        let state = self.state.take().expect("episode state finishes once");
        self.processor.finish_count_group_episode(state);
        Ok(NativeBatchOutcome {
            counts,
            calls: self.calls,
            fuel_charges,
        })
    }

    fn next_expanded(&mut self) -> Result<Option<CurrentCommand>, NativeBatchBarrier> {
        let command = self
            .processor
            .get_x_token()
            .map_err(NativeBatchBarrier::Command)?;
        self.check_command_barrier()?;
        Ok(command)
    }

    fn check_command_barrier(&mut self) -> Result<(), NativeBatchBarrier> {
        if self.processor.episode_has_pending_diagnostic() {
            return Err(NativeBatchBarrier::Required(
                NativeBatchRequiredBarrier::Diagnostic,
            ));
        }
        if self.processor.episode_has_pending_file_framing()
            || self.processor.episode_effect_pos() != self.initial_effect_pos
        {
            return Err(NativeBatchBarrier::Required(
                NativeBatchRequiredBarrier::Effect,
            ));
        }
        Ok(())
    }

    fn assign_count(&mut self) -> Result<(), NativeBatchBarrier> {
        let index = self.scan_register_index()?;
        self.processor
            .scan_optional_equals()
            .map_err(NativeBatchBarrier::Command)?;
        self.check_command_barrier()?;
        let value = self.scan_number()?;
        self.write_count(index, value);
        Ok(())
    }

    fn advance_count(&mut self) -> Result<(), NativeBatchBarrier> {
        self.expect_expanded_control(Control::Count, "advance target")?;
        let index = self.scan_register_index()?;
        let by = self
            .processor
            .scan_keyword("by")
            .map_err(NativeBatchBarrier::Command)?;
        self.check_command_barrier()?;
        if !by.value {
            return Err(NativeBatchBarrier::Malformed("advance keyword"));
        }
        let amount = self.scan_number()?;
        let value = self
            .count(index)
            .checked_add(amount)
            .ok_or(NativeBatchBarrier::ArithmeticOverflow)?;
        self.write_count(index, value);
        Ok(())
    }

    fn begin_hbox(&mut self) -> Result<(), NativeBatchBarrier> {
        if !self.pending_shipout {
            return Err(NativeBatchBarrier::Malformed("hbox outside shipout"));
        }
        let opener = self
            .next_expanded()?
            .ok_or(NativeBatchBarrier::Malformed("hbox opener"))?;
        if !matches!(
            opener.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        ) {
            return Err(NativeBatchBarrier::Malformed("hbox opener"));
        }
        self.nodes.reserve(self.expected_calls.saturating_mul(2));
        self.enter_group_raw(GroupKind::HBox);
        self.pending_shipout = false;
        self.in_hbox = true;
        Ok(())
    }

    fn begin_group(&mut self, kind: GroupKind) -> Result<(), NativeBatchBarrier> {
        if self.global_prefix {
            return Err(NativeBatchBarrier::Malformed("prefix before group"));
        }
        self.enter_group_raw(kind);
        Ok(())
    }

    fn enter_group_raw(&mut self, kind: GroupKind) {
        let state = self.state.as_mut().expect("episode state remains live");
        self.processor.episode_enter_group(state, kind);
    }

    fn end_group(&mut self, expected: GroupKind) -> Result<(), NativeBatchBarrier> {
        if self.global_prefix || self.group_depth() <= self.initial_group_depth {
            return Err(NativeBatchBarrier::Malformed("unmatched group end"));
        }
        if self.processor.episode_innermost_group_kind(self.state()) != Some(expected) {
            return Err(NativeBatchBarrier::Malformed("mismatched group end"));
        }
        let state = self.state.as_mut().expect("episode state remains live");
        self.processor
            .episode_leave_group(state, expected)
            .map_err(|_| NativeBatchBarrier::Malformed("mismatched group end"))
    }

    fn emit_kern(&mut self) -> Result<(), NativeBatchBarrier> {
        if !self.in_hbox {
            return Err(NativeBatchBarrier::Malformed("kern outside hbox"));
        }
        let amount = self
            .processor
            .scan_dimension()
            .map_err(NativeBatchBarrier::Command)?;
        self.check_command_barrier()?;
        self.nodes.kern(amount.value.raw());
        Ok(())
    }

    fn scan_register_index(&mut self) -> Result<u8, NativeBatchBarrier> {
        u8::try_from(self.scan_number()?)
            .map_err(|_| NativeBatchBarrier::Malformed("count register index"))
    }

    fn scan_number(&mut self) -> Result<i32, NativeBatchBarrier> {
        let scanned = self
            .processor
            .scan_integer()
            .map_err(NativeBatchBarrier::Command)?;
        self.check_command_barrier()?;
        Ok(scanned.value)
    }

    fn write_count(&mut self, index: u8, value: i32) {
        let state = self.state.as_mut().expect("episode state remains live");
        self.processor
            .episode_set_count(state, index, value, self.global_prefix);
        self.global_prefix = false;
    }

    fn count(&self, index: u8) -> i32 {
        self.processor.episode_count(self.state(), index)
    }

    fn group_depth(&self) -> u32 {
        self.processor.episode_group_depth(self.state())
    }

    fn expect_expanded_control(
        &mut self,
        expected: Control,
        context: &'static str,
    ) -> Result<(), NativeBatchBarrier> {
        let actual = self
            .next_expanded()?
            .as_ref()
            .and_then(Control::from_command);
        (actual == Some(expected))
            .then_some(())
            .ok_or(NativeBatchBarrier::Malformed(context))
    }
}

fn command_character(command: &CurrentCommand) -> Option<(u8, Catcode)> {
    let Meaning::CharToken { ch, cat } = command.meaning() else {
        return None;
    };
    u8::try_from(u32::from(ch)).ok().map(|ch| (ch, cat))
}

fn required_barrier(meaning: Meaning) -> Option<NativeBatchRequiredBarrier> {
    let Meaning::UnexpandablePrimitive(primitive) = meaning else {
        return None;
    };
    Some(match primitive {
        UnexpandablePrimitive::Font
        | UnexpandablePrimitive::OpenIn
        | UnexpandablePrimitive::CloseIn
        | UnexpandablePrimitive::Read
        | UnexpandablePrimitive::ReadLine => NativeBatchRequiredBarrier::Resource,
        UnexpandablePrimitive::Message
        | UnexpandablePrimitive::Write
        | UnexpandablePrimitive::Immediate
        | UnexpandablePrimitive::OpenOut
        | UnexpandablePrimitive::CloseOut
        | UnexpandablePrimitive::Special => NativeBatchRequiredBarrier::Effect,
        UnexpandablePrimitive::Show
        | UnexpandablePrimitive::ShowBox
        | UnexpandablePrimitive::ShowThe
        | UnexpandablePrimitive::ShowTokens
        | UnexpandablePrimitive::ShowLists
        | UnexpandablePrimitive::ErrMessage => NativeBatchRequiredBarrier::Diagnostic,
        UnexpandablePrimitive::Dump => NativeBatchRequiredBarrier::Format,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
