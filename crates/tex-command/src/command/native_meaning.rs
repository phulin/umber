//! Demanded semantic projections of the actual delivered native meaning.

use super::CurrentCommand;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord, StaticCommandClass,
    UnexpandablePrimitive,
};
use tex_state::token::Catcode;

impl<G> CurrentCommand<G> {
    #[inline(always)]
    /// Borrows the already-delivered meaning for a genuine preflight consumer.
    #[must_use]
    pub const fn native_meaning(&self) -> &MeaningWord<G> {
        &self.command
    }

    pub(crate) const fn flags(&self) -> MeaningFlags {
        match self.command.macro_flags() {
            Some(flags) => flags,
            None => MeaningFlags::EMPTY,
        }
    }

    #[inline(always)]
    pub(crate) const fn expandable_primitive(&self) -> Option<ExpandablePrimitive> {
        if let Some(word) = self.command.scalar_word()
            && matches!(
                Meaning::runtime_word_class(word),
                StaticCommandClass::Expandable
            )
        {
            ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) const fn unexpandable_primitive(&self) -> Option<UnexpandablePrimitive> {
        if let Some(word) = self.command.scalar_word()
            && matches!(
                Meaning::runtime_word_class(word),
                StaticCommandClass::Unexpandable
            )
        {
            UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
        } else {
            None
        }
    }

    /// Decodes only a static meaning at its semantic consumer. Font and macro
    /// capabilities keep their native forms and have no scalar projection.
    #[inline(always)]
    pub(crate) fn static_meaning(&self) -> Option<Meaning> {
        self.command.scalar_word().map(Meaning::from_runtime_word)
    }

    #[inline(always)]
    pub(crate) fn character(&self) -> Option<(char, Catcode)> {
        self.command
            .scalar_word()
            .and_then(Meaning::runtime_word_character)
    }

    pub(crate) fn character_catcode(&self) -> Option<Catcode> {
        self.character().map(|(_, cat)| cat)
    }

    /// TeX82 §26's numeric character value also includes `\chardef`.
    #[cfg(test)]
    pub(crate) fn character_value(&self) -> Option<char> {
        match self.static_meaning() {
            Some(Meaning::CharGiven(ch) | Meaning::CharToken { ch, .. }) => Some(ch),
            _ => None,
        }
    }

    /// TeX82 §1038 continues only its actual text-character commands.
    pub(crate) fn is_main_loop_character(&self) -> bool {
        matches!(
            self.static_meaning(),
            Some(
                Meaning::CharGiven(_)
                    | Meaning::CharToken {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    }
            )
        )
    }
}
