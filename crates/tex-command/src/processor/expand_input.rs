//! Source-oriented expansion primitives.

use tex_state::env::banks::IntParam;
use tex_state::token::{Token, TracedTokenWord};

use crate::input::{
    BackedUpToken, BackupTreatment, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    TokenBehavior,
};
use crate::observation::{
    CommandObservation, EffectRecord, InputReason, InputRecord, InputTransition,
};
use crate::{
    CommandError, CurrentCommand, RegisteredSourceKind, SourceNameClass, SourceRegistration,
};

use super::CommandProcessor;

impl<G> CommandProcessor<'_, '_, G> {
    /// e-TeX 2.6 etex.ch §53a `pseudo_start`.
    pub(super) fn expand_scantokens(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "scantokens",
        })?;
        let mut text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        let newline = self.state.int_param(IntParam::NEWLINE_CHAR);
        if let Some(newline) = char::from_u32(u32::try_from(newline).unwrap_or(u32::MAX))
            && newline != '\n'
        {
            text = text
                .chars()
                .map(|ch| if ch == newline { '\n' } else { ch })
                .collect();
        }
        // etex.ch appends one sentinel space before splitting the string.
        // The pseudo-input representation is line-oriented, so a final LF
        // expresses that final record without becoming source text itself.
        text.push('\n');
        let every_eof = self
            .state
            .token_parameter(tex_state::env::banks::TokParam::EVERY_EOF)
            .expect("everyeof is an admitted token parameter");
        let tracing_scantokens = self.state.int_param(IntParam::TRACING_SCAN_TOKENS);
        let open_depths = self.capture_source_open_depths();
        self.invalidate_delivery_freshness();
        let (level, framing_name) = self
            .command
            .open_scantokens(
                SourceRegistration::new(RegisteredSourceKind::Generated, text.into_bytes())
                    .with_role(crate::SourceRole::GeneratedInput),
                every_eof,
                scantokens_numeric_name(tracing_scantokens),
                open_depths,
            )
            .map_err(|_| CommandError::input_invariant())?;
        if let Some(name) = framing_name {
            self.state.print_file_open(&name);
        }
        let source = self
            .command
            .active_source_snapshot()
            .ok_or(CommandError::input_invariant())?;
        // e-TeX 2.6 etex.ch §53a assigns `name=19` while
        // `\tracingscantokens>0`, and `name=18` otherwise. TeX82 §48's
        // initial character strings render those names as `^^S` and `^^R`.
        let source_name = scantokens_source_name(tracing_scantokens);
        let source_id = source.id;
        self.observe(CommandObservation::GeneratedSource(
            crate::GeneratedSourceRecord {
                name: source_name.to_owned(),
                source,
            },
        ));
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Source,
            // e-TeX 2.6 etex.ch §53a `pseudo_start` first calls
            // `begin_file_reading`, which establishes and observes the new
            // level while its §328 default is still `name=0`. Only after
            // that transition does e-TeX assign the pseudo-file name used
            // during tokenization and retirement. The level remains
            // file-like in command state, but its push is the transient
            // terminal-class transition the reference engine performs.
            source_name: Some(SourceNameClass::Terminal),
            source: Some(source_id),
            level: level.0,
            position: 0,
        }));
        Ok(())
    }

    pub(super) fn expand_input(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        if self.command.name_in_progress() {
            // TeX82 §§378/527 call §378's `insert_relax`: two distinct
            // `back_input` operations first restore the recursively
            // encountered `\input`, then place inaccessible `frozen_relax`
            // above it and retype only that second level as `inserted`. The
            // distinction is observable after the relax terminates the
            // active filename scan: its depleted inserted level retires, so
            // a diagnostic on the restored command says `<recently read>`.
            let opener_origin = opener.origin();
            self.back_input(opener)?;
            let frozen_relax = TracedTokenWord::pack(Token::frozen_relax(), opener_origin);
            let level = self.command.push_token_level(
                PackedTokenSpanHandle::backed_up([BackedUpToken {
                    spelling: frozen_relax,
                }]),
                TokenBehavior::BackedUp(BackupTreatment::Ordinary),
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
            self.observe_inserted_token_recovery(level, Token::frozen_relax());
            return Ok(());
        }
        let _input = self
            .open_registered_input()
            .map_err(|error| error.at_origin_unless_resource(opener.origin()))?;
        observe!(
            self,
            CommandObservation::Effect(EffectRecord {
                kind: crate::ObservationEffectKind::Input,
                channel: _input.file_name.packed(),
                value: crate::ObservationValue::None,
                source: Some(crate::observation::OpenedSourceSnapshot {
                    id: _input.source,
                    bytes: _input.bytes,
                }),
            }),
        );
        let _ = opener;
        Ok(())
    }

    pub(super) fn expand_endinput(&mut self) -> Result<(), CommandError> {
        self.invalidate_delivery_freshness();
        self.command
            .end_current_source_after_current_line()
            .then_some(())
            .ok_or(CommandError::input_invariant())
    }
}

/// e-TeX 2.6 etex.ch §53a's two pseudo-file names, rendered through TeX82
/// §48's initial character strings.
fn scantokens_source_name(tracing_scantokens: i32) -> &'static str {
    if tracing_scantokens > 0 { "^^S" } else { "^^R" }
}

fn scantokens_numeric_name(tracing_scantokens: i32) -> u8 {
    if tracing_scantokens > 0 { 19 } else { 18 }
}
