use std::sync::Arc;

use super::{Case, Delivery, Storage};
use tex_command::{CommandState, RegisteredSourceKind, SourceRegistration};
use tex_state::Universe;
use tex_state::env::AssignmentScope;
use tex_state::interner::Symbol;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

const DEFAULT_INTERNER_SLOTS: u32 = 4_096;
const DEFAULT_INTERNER_HASH: u32 = 4_096;
const DEFAULT_INTERNER_BYTES: u32 = 1 << 20;

#[derive(Clone, Copy, Debug)]
pub(super) struct Workload {
    source_text: &'static str,
    raw_source_text: &'static str,
    macro_name: Option<&'static str>,
    parameter_text: &'static [TokenSpec],
    replacement_text: &'static [TokenSpec],
    raw_output: OutputToken,
    expanded_output: OutputToken,
    pub(crate) macro_calls_per_operation: usize,
    pub(crate) body_tokens_per_operation: usize,
    pub(crate) argument_tokens_per_operation: usize,
    pub(crate) delimiter_tokens_per_operation: usize,
    pub(crate) source_words_per_operation: usize,
    pub(crate) stored_words_per_operation: usize,
    pub(crate) raw_source_words_per_operation: usize,
    pub(crate) raw_stored_words_per_operation: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Denominators {
    pub(crate) source_words_per_operation: usize,
    pub(crate) stored_words_per_operation: usize,
    pub(crate) input_tokens_per_operation: usize,
    pub(crate) token_work_per_operation: usize,
    pub(crate) macro_calls_per_operation: usize,
    pub(crate) body_tokens_per_operation: usize,
    pub(crate) argument_tokens_per_operation: usize,
    pub(crate) delimiter_tokens_per_operation: usize,
}

impl Workload {
    pub(crate) fn denominators(self, storage: Storage, delivery: Delivery) -> Denominators {
        let expanded = matches!(delivery, Delivery::Expanded);
        let source_words_per_operation = if expanded {
            self.source_words_per_operation
        } else {
            self.raw_source_words_per_operation
        };
        let stored_words_per_operation = if expanded {
            self.stored_words_per_operation
        } else {
            self.raw_stored_words_per_operation
        };
        let input_tokens_per_operation = match storage {
            Storage::Source => source_words_per_operation,
            Storage::Stored => stored_words_per_operation,
        };
        let macro_calls_per_operation = usize::from(expanded) * self.macro_calls_per_operation;
        let body_tokens_per_operation = usize::from(expanded) * self.body_tokens_per_operation;
        let argument_tokens_per_operation =
            usize::from(expanded) * self.argument_tokens_per_operation;
        let delimiter_tokens_per_operation =
            usize::from(expanded) * self.delimiter_tokens_per_operation;
        Denominators {
            source_words_per_operation,
            stored_words_per_operation,
            input_tokens_per_operation,
            token_work_per_operation: input_tokens_per_operation
                .saturating_add(body_tokens_per_operation),
            macro_calls_per_operation,
            body_tokens_per_operation,
            argument_tokens_per_operation,
            delimiter_tokens_per_operation,
        }
    }

    pub(crate) fn input_text(self, delivery: Delivery) -> &'static str {
        if matches!(delivery, Delivery::Expanded) {
            self.source_text
        } else {
            self.raw_source_text
        }
    }

    pub(crate) fn output(self, delivery: Delivery) -> OutputToken {
        if matches!(delivery, Delivery::Expanded) {
            self.expanded_output
        } else {
            self.raw_output
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TokenSpec {
    Char(char, Catcode),
    Parameter(u8),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum OutputToken {
    Char(char),
    ControlSequence,
}

impl OutputToken {
    pub(crate) const fn checksum(self) -> u64 {
        match self {
            Self::Char(ch) => ch as u64,
            Self::ControlSequence => 0xC000_0001,
        }
    }
}

pub(crate) fn workload(case: Case) -> Workload {
    match case {
        Case::Plain => Workload {
            source_text: "x",
            raw_source_text: "x",
            macro_name: None,
            parameter_text: &[],
            replacement_text: &[],
            raw_output: OutputToken::Char('x'),
            expanded_output: OutputToken::Char('x'),
            macro_calls_per_operation: 0,
            body_tokens_per_operation: 0,
            argument_tokens_per_operation: 0,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 1,
            stored_words_per_operation: 1,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::EmptyMacro => Workload {
            source_text: "\\mempty x",
            raw_source_text: "\\mempty",
            macro_name: Some("mempty"),
            parameter_text: &[],
            replacement_text: &[],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('x'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 0,
            argument_tokens_per_operation: 0,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 2,
            stored_words_per_operation: 2,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::NonemptyMacro => Workload {
            source_text: "\\mlit",
            raw_source_text: "\\mlit",
            macro_name: Some("mlit"),
            parameter_text: &[],
            replacement_text: &[TokenSpec::Char('z', Catcode::Letter)],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('z'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 1,
            argument_tokens_per_operation: 0,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 1,
            stored_words_per_operation: 1,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::ParameterizedEmpty => Workload {
            source_text: "\\mpempty{a}x",
            raw_source_text: "\\mpempty",
            macro_name: Some("mpempty"),
            parameter_text: &[TokenSpec::Parameter(1)],
            replacement_text: &[],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('x'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 0,
            argument_tokens_per_operation: 1,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 5,
            stored_words_per_operation: 5,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::ParameterizedIdentity => Workload {
            source_text: "\\mpid a",
            raw_source_text: "\\mpid",
            macro_name: Some("mpid"),
            parameter_text: &[TokenSpec::Parameter(1)],
            replacement_text: &[TokenSpec::Parameter(1)],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('a'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 1,
            argument_tokens_per_operation: 1,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 2,
            stored_words_per_operation: 2,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::BalancedArgument => Workload {
            source_text: "\\mpbal{a{b}}",
            raw_source_text: "\\mpbal",
            macro_name: Some("mpbal"),
            parameter_text: &[TokenSpec::Parameter(1)],
            replacement_text: &[TokenSpec::Char('z', Catcode::Letter)],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('z'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 1,
            argument_tokens_per_operation: 4,
            delimiter_tokens_per_operation: 0,
            source_words_per_operation: 7,
            stored_words_per_operation: 7,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
        Case::DelimitedNestedArgument => Workload {
            source_text: "\\mpdel{a{b}}|",
            raw_source_text: "\\mpdel",
            macro_name: Some("mpdel"),
            parameter_text: &[
                TokenSpec::Parameter(1),
                TokenSpec::Char('|', Catcode::Other),
            ],
            replacement_text: &[TokenSpec::Char('z', Catcode::Letter)],
            raw_output: OutputToken::ControlSequence,
            expanded_output: OutputToken::Char('z'),
            macro_calls_per_operation: 1,
            body_tokens_per_operation: 1,
            argument_tokens_per_operation: 4,
            delimiter_tokens_per_operation: 1,
            source_words_per_operation: 8,
            stored_words_per_operation: 8,
            raw_source_words_per_operation: 1,
            raw_stored_words_per_operation: 1,
        },
    }
}

pub(super) fn install_macro<G>(universe: &mut Universe<G>, workload: Workload) -> Option<Symbol> {
    let Some(name) = workload.macro_name else {
        return None;
    };
    let symbol_id = universe.intern(name).expect("matrix macro name");
    let parameters = token_specs(workload.parameter_text);
    let replacement = token_specs(workload.replacement_text);
    let definition = universe
        .allocate_definition(&parameters, &replacement)
        .expect("matrix macro definition");
    universe
        .assign_meaning(
            symbol_id,
            MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
            AssignmentScope::Global,
        )
        .expect("matrix macro meaning");
    Some(symbol_id.symbol())
}

pub(super) fn build_source_text(
    workload: Workload,
    delivery: Delivery,
    operations: usize,
) -> String {
    // A repeated raw control word is terminated by the following backslash,
    // so the raw stream stays at one semantic token per operation without a
    // measured separator token. Expanded streams contain their own argument
    // and result tokens and use the same source bytes as their stored words.
    let unit = workload.input_text(delivery);
    unit.repeat(operations)
}

pub(super) fn build_stored_words(
    workload: Workload,
    macro_symbol: Option<Symbol>,
    delivery: Delivery,
    operations: usize,
) -> Vec<TokenWord> {
    let capacity = workload
        .denominators(Storage::Stored, delivery)
        .stored_words_per_operation
        .saturating_mul(operations);
    let mut words = Vec::with_capacity(capacity);
    for _ in 0..operations {
        if !matches!(delivery, Delivery::Expanded) {
            if let Some(symbol) = macro_symbol {
                words.push(TokenWord::pack(Token::Cs(symbol)));
            } else {
                words.push(char_word('x'));
            }
            continue;
        }
        if let Some(symbol) = macro_symbol {
            words.push(TokenWord::pack(Token::Cs(symbol)));
        }
        match workload.macro_name {
            None => words.push(char_word('x')),
            Some("mempty") => words.push(char_word('x')),
            Some("mlit") => {}
            Some("mpempty") => {
                words.extend([begin_group(), char_word('a'), end_group(), char_word('x')]);
            }
            Some("mpid") => words.push(char_word('a')),
            Some("mpbal") => words.extend([
                begin_group(),
                char_word('a'),
                begin_group(),
                char_word('b'),
                end_group(),
                end_group(),
            ]),
            Some("mpdel") => words.extend([
                begin_group(),
                char_word('a'),
                begin_group(),
                char_word('b'),
                end_group(),
                end_group(),
                TokenWord::pack(Token::Char {
                    ch: '|',
                    cat: Catcode::Other,
                }),
            ]),
            Some(other) => panic!("unknown matrix macro {other}"),
        }
    }
    words
}

pub(super) fn install_benchmark_catcodes<G>(universe: &mut Universe<G>) {
    let mut context = universe
        .command_context()
        .expect("matrix command context for catcodes");
    for (character, catcode) in [
        ('{', Catcode::BeginGroup),
        ('}', Catcode::EndGroup),
        ('#', Catcode::Parameter),
    ] {
        context
            .assign_code(
                tex_state::CodeTableKind::Catcode,
                character,
                i64::from(catcode as u8),
                AssignmentScope::Global,
            )
            .expect("matrix fixture category code");
    }
}

fn token_specs(specs: &[TokenSpec]) -> Vec<TokenWord> {
    specs
        .iter()
        .map(|spec| match spec {
            TokenSpec::Char(ch, cat) => TokenWord::pack(Token::Char { ch: *ch, cat: *cat }),
            TokenSpec::Parameter(index) => TokenWord::pack(Token::param(*index)),
        })
        .collect()
}

pub(super) fn open_source<G>(command: &mut CommandState<G>, source: &str) {
    let registered = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ))
        .expect("matrix source registration");
    command
        .open_registered_source(registered)
        .expect("matrix source opening");
}

fn char_word(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Letter,
    })
}

fn begin_group() -> TokenWord {
    TokenWord::pack(Token::Char {
        ch: '{',
        cat: Catcode::BeginGroup,
    })
}

fn end_group() -> TokenWord {
    TokenWord::pack(Token::Char {
        ch: '}',
        cat: Catcode::EndGroup,
    })
}

pub(super) fn with_universe<R>(
    run: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = tex_state::interner::InternerBudget::new(
        DEFAULT_INTERNER_SLOTS,
        DEFAULT_INTERNER_HASH,
        DEFAULT_INTERNER_BYTES,
    )
    .expect("matrix interner budget");
    tex_state::with_universe(budget, run).expect("matrix universe")
}
