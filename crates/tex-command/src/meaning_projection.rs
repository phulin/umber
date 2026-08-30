//! Focused structural harness for borrowed current-meaning projection.

use tex_state::CommandContext;
use tex_state::env::AssignmentScope;
use tex_state::font::NULL_FONT;
use tex_state::interner::Symbol;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord, ResolvedMeaning,
};
use tex_state::token::{Catcode, OriginId, Token, TokenWord};

use crate::command::{CurrentCommand, command_ownership_counters};

const MIXED_MEANINGS: usize = 9;
const MACROS_PER_ROUND: u64 = 2;

/// Focused mixed-meaning fixture for the borrowed projection boundary.
pub struct MeaningProjectionBenchmark<G> {
    words: [TokenWord; MIXED_MEANINGS],
    command: CurrentCommand<G>,
    sequence: u64,
}

/// Exact structural receipt from one mixed-meaning projection run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeaningProjectionReceipt {
    pub resolved_meanings: u64,
    pub table_probes: u64,
    pub tag_decodes: u64,
    pub macro_meanings: u64,
    pub macro_owner_resolutions: u64,
    pub duplicate_owner_resolutions: u64,
    pub whole_meaning_copies: u64,
    pub whole_command_copies: u64,
    pub checksum: u64,
}

impl<G> MeaningProjectionBenchmark<G> {
    /// Installs undefined, primitive, macro/alias, register, font, static
    /// alias, and active-character rows in one admitted meaning table.
    pub fn new(universe: &mut tex_state::Universe<G>) -> Self {
        let symbols: [Symbol; 8] = [
            "projectionundefined",
            "projectionprimitive",
            "projectionmacro",
            "projectionregister",
            "projectionparameter",
            "projectionfont",
            "projectionalias",
            "projectionstatic",
        ]
        .map(|name| {
            universe
                .intern(name)
                .expect("projection fixture symbol")
                .symbol()
        });
        let definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'm',
                    cat: Catcode::Letter,
                })],
            )
            .expect("projection fixture definition");
        let rows = [
            MeaningWord::from_static(Meaning::Undefined),
            MeaningWord::from_static(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::ExpandAfter,
            )),
            MeaningWord::macro_definition(MeaningFlags::LONG, definition.clone()),
            MeaningWord::from_static(Meaning::CountRegister(32_767)),
            MeaningWord::from_static(Meaning::DimenParam(17)),
            MeaningWord::from_static(Meaning::Font(NULL_FONT)),
            MeaningWord::macro_definition(MeaningFlags::LONG, definition),
            MeaningWord::from_static(Meaning::CharGiven('Z')),
        ];
        for (symbol, meaning) in symbols.into_iter().zip(rows) {
            universe
                .assign_meaning(
                    universe
                        .qualify_symbol(symbol)
                        .expect("projection symbol remains admitted"),
                    meaning,
                    AssignmentScope::Global,
                )
                .expect("projection fixture meaning");
        }
        let active = universe
            .intern_active_character('~')
            .expect("projection active character");
        universe
            .assign_meaning(
                active,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("projection active meaning");

        let mut words = [TokenWord::pack(Token::Cs(symbols[0])); MIXED_MEANINGS];
        for (destination, symbol) in words[..8].iter_mut().zip(symbols) {
            *destination = TokenWord::pack(Token::Cs(symbol));
        }
        words[8] = TokenWord::pack(Token::Char {
            ch: '~',
            cat: Catcode::Active,
        });
        Self {
            words,
            command: CurrentCommand::empty(),
            sequence: 0,
        }
    }

    /// Projects `rounds * 9` meanings through one reusable command slot.
    pub fn run(&mut self, state: &CommandContext<'_, G>, rounds: u32) -> MeaningProjectionReceipt {
        let before_meaning = tex_state::meaning::meaning_projection_counters();
        let before_command = command_ownership_counters();
        let mut checksum = 0_u64;
        for _ in 0..rounds {
            for (index, word) in self.words.iter().copied().enumerate() {
                self.sequence = self.sequence.wrapping_add(1);
                let (resolved, resolution) = self
                    .command
                    .empty_for_raw_delivery()
                    .write_resolved_delivery(
                        std::hint::black_box(word),
                        OriginId::UNKNOWN,
                        1,
                        self.sequence,
                        self.sequence,
                        None,
                        None,
                        false,
                        None,
                        false,
                        state,
                    );
                assert!(resolution.meaning_lookup());
                let semantic = match (index, resolved.as_ref().meaning_ref()) {
                    (0, ResolvedMeaning::Static(Meaning::Undefined)) => 1,
                    (
                        1,
                        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                            ExpandablePrimitive::ExpandAfter,
                        )),
                    ) => 2,
                    (2 | 6, ResolvedMeaning::Macro { flags, definition })
                        if *flags == MeaningFlags::LONG
                            && definition.replacement_word(0)
                                == Some(TokenWord::pack(Token::Char {
                                    ch: 'm',
                                    cat: Catcode::Letter,
                                })) =>
                    {
                        3
                    }
                    (3, ResolvedMeaning::Static(Meaning::CountRegister(32_767))) => 4,
                    (4, ResolvedMeaning::Static(Meaning::DimenParam(17))) => 5,
                    (5, ResolvedMeaning::Static(Meaning::Font(font))) if *font == NULL_FONT => 6,
                    (7, ResolvedMeaning::Static(Meaning::CharGiven('Z'))) => 7,
                    (8, ResolvedMeaning::Static(Meaning::CharGiven('A'))) => 8,
                    _ => panic!("mixed meaning projection changed semantics at row {index}"),
                };
                checksum = checksum.wrapping_add(semantic);
            }
        }
        let after_meaning = tex_state::meaning::meaning_projection_counters();
        let after_command = command_ownership_counters();
        let resolved_meanings = u64::from(rounds) * MIXED_MEANINGS as u64;
        let macro_meanings = u64::from(rounds) * MACROS_PER_ROUND;
        let macro_owner_resolutions = after_meaning
            .macro_owner_resolutions
            .saturating_sub(before_meaning.macro_owner_resolutions);
        MeaningProjectionReceipt {
            resolved_meanings,
            table_probes: after_meaning
                .table_probes
                .saturating_sub(before_meaning.table_probes),
            tag_decodes: after_meaning
                .tag_decodes
                .saturating_sub(before_meaning.tag_decodes),
            macro_meanings,
            macro_owner_resolutions,
            duplicate_owner_resolutions: macro_owner_resolutions.saturating_sub(macro_meanings),
            whole_meaning_copies: after_meaning
                .meaning_word_clones
                .saturating_sub(before_meaning.meaning_word_clones)
                .saturating_add(
                    after_meaning
                        .resolved_meaning_clones
                        .saturating_sub(before_meaning.resolved_meaning_clones),
                ),
            whole_command_copies: after_command
                .clones
                .saturating_sub(before_command.clones)
                .saturating_add(
                    after_command
                        .backup_copies
                        .saturating_sub(before_command.backup_copies),
                ),
            checksum,
        }
    }
}
