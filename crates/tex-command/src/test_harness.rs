//! Fresh, shallow state for crate-internal processor and scanner tests.
//!
//! tex.web §75 starts a job in `error_stop_mode`, and §82 enters §83's dialog
//! on that alone. A memory `World` with no terminal lines pushed into it is a
//! terminal at end of file, which §71 answers with
//! `fatal_error("End of file on the terminal!")` -- so a scanner test that
//! raises any recoverable error would end its job on the dialog rather than
//! recover and keep scanning.
//!
//! That is the right behavior for an interactive job, and it is what
//! `umber2-er8c` restored. It is simply not what these tests are about: they
//! exercise §§413-460's scanners and their recoveries, not the terminal. So
//! they run the job the way a `\nonstopmode` document does, which is also
//! what the minifixture corpus does for the same reason.

use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::world::PrintSink;
use tex_state::{EffectRecord, Universe};

use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandProfile, CommandState,
};

/// A [`Universe`] in `\nonstopmode`, with §75's dialog off.
#[must_use]
pub(crate) fn universe() -> Universe {
    let mut universe = Universe::new();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}

/// [`universe`] with plain TeX's category codes already installed.
#[must_use]
pub(crate) fn universe_with_plain_catcodes() -> Universe {
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}

/// One fresh processor episode without hiding any mutable engine state.
pub(crate) struct ProcessorScenario {
    pub(crate) command: CommandState,
    pub(crate) universe: Universe,
    pub(crate) capabilities: CommandHostCapabilities,
}

impl ProcessorScenario {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::with_profile(CommandProfile::TEX82)
    }

    #[must_use]
    pub(crate) fn plain() -> Self {
        Self {
            command: CommandState::default(),
            universe: universe_with_plain_catcodes(),
            capabilities: CommandHostCapabilities::default(),
        }
    }

    #[must_use]
    pub(crate) fn with_profile(profile: CommandProfile) -> Self {
        Self {
            command: CommandState::new(profile),
            universe: universe(),
            capabilities: CommandHostCapabilities::default(),
        }
    }

    pub(crate) fn push(&mut self, tokens: impl IntoIterator<Item = Token>) {
        push(&mut self.command, tokens);
    }

    pub(crate) fn processor(&mut self) -> CommandProcessor<'_> {
        processor(
            &mut self.command,
            &mut self.universe,
            &mut self.capabilities,
        )
    }

    #[must_use]
    pub(crate) fn diagnostic_text(&self) -> String {
        diagnostic_text(&self.universe)
    }
}

impl Default for ProcessorScenario {
    fn default() -> Self {
        Self::new()
    }
}

/// A fresh processor scenario with its own committed-observation recorder.
#[derive(Default)]
pub(crate) struct ScannerRig {
    pub(crate) scenario: ProcessorScenario,
    pub(crate) recorder: Recorder,
}

impl ScannerRig {
    #[must_use]
    pub(crate) fn plain() -> Self {
        Self {
            scenario: ProcessorScenario::plain(),
            recorder: Recorder::default(),
        }
    }

    pub(crate) fn processor(&mut self) -> CommandProcessor<'_> {
        CommandProcessor::new(
            &mut self.scenario.command,
            self.scenario.universe.command_context(),
            CommandHostContext::new(&mut self.scenario.capabilities),
        )
        .with_observer(&mut self.recorder)
    }
}

#[derive(Default)]
pub(crate) struct Recorder(pub(crate) Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[must_use]
pub(crate) fn traced(token: Token) -> TracedTokenWord {
    TracedTokenWord::pack(token, OriginId::UNKNOWN)
}

pub(crate) fn push(command: &mut CommandState, tokens: impl IntoIterator<Item = Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

pub(crate) fn processor<'a>(
    command: &'a mut CommandState,
    universe: &'a mut Universe,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
        universe.command_context(),
        CommandHostContext::new(capabilities),
    )
}

#[must_use]
pub(crate) fn plain_text_tokens(text: &str) -> Vec<Token> {
    text.chars()
        .map(|ch| Token::Char {
            ch,
            cat: match ch {
                '{' => Catcode::BeginGroup,
                '}' => Catcode::EndGroup,
                ' ' => Catcode::Space,
                'a'..='z' | 'A'..='Z' => Catcode::Letter,
                _ => Catcode::Other,
            },
        })
        .collect()
}

#[must_use]
pub(crate) fn diagnostic_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
