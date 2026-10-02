//! Demanded semantic projections of the actual delivered native meaning.

use super::{CommandClass, CurrentCommand};
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord, StaticCommandClass,
    UnexpandablePrimitive,
};
use tex_state::token::Catcode;

impl<G> CurrentCommand<G> {
    #[inline(always)]
    pub(crate) const fn class(&self) -> CommandClass {
        match self.command {
            MeaningWord::Macro { .. } => CommandClass::Macro,
            MeaningWord::Font(_) => CommandClass::Font,
            MeaningWord::Static(word) => match Meaning::runtime_word_class(word) {
                StaticCommandClass::Undefined => CommandClass::Undefined,
                StaticCommandClass::Relax => CommandClass::Relax,
                StaticCommandClass::Character => CommandClass::Character,
                StaticCommandClass::Expandable => CommandClass::Expandable,
                StaticCommandClass::EndV => CommandClass::EndV,
                StaticCommandClass::Unexpandable => CommandClass::Unexpandable,
                StaticCommandClass::Value => CommandClass::Value,
            },
        }
    }

    pub(crate) const fn flags(&self) -> MeaningFlags {
        match self.command {
            MeaningWord::Macro { flags, .. } => flags,
            _ => MeaningFlags::EMPTY,
        }
    }

    #[inline(always)]
    pub(crate) const fn expandable_primitive(&self) -> Option<ExpandablePrimitive> {
        match self.command {
            MeaningWord::Static(word)
                if matches!(
                    Meaning::runtime_word_class(word),
                    StaticCommandClass::Expandable
                ) =>
            {
                ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
            }
            _ => None,
        }
    }

    #[inline(always)]
    pub(crate) const fn unexpandable_primitive(&self) -> Option<UnexpandablePrimitive> {
        match self.command {
            MeaningWord::Static(word)
                if matches!(
                    Meaning::runtime_word_class(word),
                    StaticCommandClass::Unexpandable
                ) =>
            {
                UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
            }
            _ => None,
        }
    }

    /// Decodes only a static meaning at its semantic consumer. Font and macro
    /// capabilities keep their native forms and have no scalar projection.
    #[inline(always)]
    pub(crate) fn static_meaning(&self) -> Option<Meaning> {
        match self.command {
            MeaningWord::Static(word) => Some(Meaning::from_runtime_word(word)),
            _ => None,
        }
    }

    #[inline(always)]
    pub(crate) fn character(&self) -> Option<(char, Catcode)> {
        match self.command {
            MeaningWord::Static(word) => Meaning::runtime_word_character(word),
            _ => None,
        }
    }

    pub(crate) fn character_catcode(&self) -> Option<Catcode> {
        self.character().map(|(_, cat)| cat)
    }

    /// Literal character spelling excludes the numeric `\chardef` value.
    pub(crate) fn character_token(&self) -> Option<char> {
        self.character().map(|(ch, _)| ch)
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
