use std::sync::Arc;

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    RegisteredSourceKind, SourceRegistration,
};
use tex_state::Universe;
use tex_state::env::AssignmentScope;
use tex_state::interner::Symbol;
use tex_state::meaning::{MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TokenWord};

#[path = "command_consumer_fixture_body.rs"]
mod body;

use body::{append_source_body, append_stored_body, body_token, body_token_value};
pub(super) use body::{body_tokens, expected_body_checksum};

pub(super) const DEFAULT_TEXT_CHARS: usize = 4_096;
pub(super) const DEFAULT_BODY_WORDS: usize = 8_193;
pub(super) const DEFAULT_ITERATIONS: usize = 16;
pub(super) const DEFAULT_WARMUPS: usize = 2;
pub(super) const DEFAULT_CHAIN_ITERATIONS: usize = 256;
pub(super) const DEFAULT_MIXED_ITERATIONS: usize = 32;

pub(super) const TEXT_CHARACTER: char = 'a';
pub(super) const BODY_CHARACTER: char = 'b';
pub(super) const BODY_CONTROL_NAME: &str = "consumerbodytoken";
pub(super) const CHAIN_RESULT_CHARACTER: char = 'q';
pub(super) const SEMANTIC_SEED: u64 = 0xCBF2_9CE4_8422_2325;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Storage {
    Source,
    Stored,
}

impl Storage {
    pub(super) const ALL: [Self; 2] = [Self::Source, Self::Stored];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Stored => "stored",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkloadKind {
    LongText,
    DefinitionBody,
    ParameterizedChain,
    MixedPipeline,
}

impl WorkloadKind {
    pub(super) const ALL: [Self; 4] = [
        Self::LongText,
        Self::DefinitionBody,
        Self::ParameterizedChain,
        Self::MixedPipeline,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::LongText => "long_text",
            Self::DefinitionBody => "definition_body",
            Self::ParameterizedChain => "parameterized_chain",
            Self::MixedPipeline => "mixed_pipeline",
        }
    }

    pub(super) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub(super) const fn default_iterations(self) -> usize {
        match self {
            Self::LongText | Self::DefinitionBody => DEFAULT_ITERATIONS,
            Self::ParameterizedChain => DEFAULT_CHAIN_ITERATIONS,
            Self::MixedPipeline => DEFAULT_MIXED_ITERATIONS,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Evidence {
    pub(super) count: u64,
    pub(super) checksum: u64,
    pub(super) hash: u64,
}

impl Evidence {
    pub(super) const fn seeded() -> Self {
        Self {
            count: 0,
            checksum: 0,
            hash: SEMANTIC_SEED,
        }
    }

    pub(super) fn absorb(&mut self, ch: char) {
        self.count = self.count.saturating_add(1);
        self.checksum = self.checksum.wrapping_add(ch as u64);
        self.hash = self.hash.rotate_left(7) ^ (ch as u64);
    }

    pub(super) fn absorb_command(&mut self, ch: char, cat: Catcode, control_sequence: bool) {
        self.count = self.count.saturating_add(1);
        self.checksum = self.checksum.wrapping_add(ch as u64);
        self.hash = self.hash.rotate_left(7)
            ^ (ch as u64)
            ^ ((cat as u64) << 32)
            ^ u64::from(control_sequence);
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Denominators {
    pub(super) source_words_per_operation: usize,
    pub(super) stored_words_per_operation: usize,
    pub(super) input_tokens_per_operation: usize,
    pub(super) token_work_per_operation: usize,
    pub(super) text_characters_per_operation: usize,
    pub(super) macro_calls_per_operation: usize,
    pub(super) definition_calls_per_operation: usize,
    pub(super) definition_body_words_per_operation: usize,
}

pub(super) fn denominators(
    kind: WorkloadKind,
    text_chars: usize,
    body_words: usize,
    storage: Storage,
) -> Denominators {
    let (source_words, stored_words, token_work, text, macros, definitions, body) = match kind {
        WorkloadKind::LongText => (text_chars, text_chars, text_chars, text_chars, 0, 0, 0),
        WorkloadKind::DefinitionBody => {
            let input = body_words.saturating_add(5);
            (input, input, input, 0, 0, 1, body_words)
        }
        WorkloadKind::ParameterizedChain => (4, 4, 13, 0, 3, 0, 0),
        WorkloadKind::MixedPipeline => {
            let definition_input = body_words.saturating_add(5);
            let macro_work = 13;
            let input = text_chars
                .saturating_add(4)
                .saturating_add(definition_input);
            (
                input,
                input,
                text_chars
                    .saturating_add(macro_work)
                    .saturating_add(definition_input),
                text_chars,
                3,
                1,
                body_words,
            )
        }
    };
    Denominators {
        source_words_per_operation: source_words,
        stored_words_per_operation: stored_words,
        input_tokens_per_operation: match storage {
            Storage::Source => source_words,
            Storage::Stored => stored_words,
        },
        token_work_per_operation: token_work,
        text_characters_per_operation: text,
        macro_calls_per_operation: macros,
        definition_calls_per_operation: definitions,
        definition_body_words_per_operation: body,
    }
}

pub(super) fn expected_text(chars: usize, iterations: usize) -> Evidence {
    let mut evidence = Evidence::seeded();
    for _ in 0..iterations {
        for _ in 0..chars {
            evidence.absorb(TEXT_CHARACTER);
        }
    }
    evidence
}

pub(super) fn expected_chain(iterations: usize) -> Evidence {
    let mut evidence = Evidence::seeded();
    for _ in 0..iterations {
        evidence.absorb_command(CHAIN_RESULT_CHARACTER, Catcode::Letter, false);
    }
    evidence
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChainSymbols {
    pub(super) first: Symbol,
    second: Symbol,
    third: Symbol,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DefinitionSymbols {
    pub(super) target: Symbol,
    pub(super) body_control: Symbol,
}

pub(super) fn install_definition_symbol<G>(universe: &mut Universe<G>) -> DefinitionSymbols {
    DefinitionSymbols {
        target: universe
            .intern("consumerdefinition")
            .expect("definition target")
            .symbol(),
        body_control: universe
            .intern(BODY_CONTROL_NAME)
            .expect("definition body control")
            .symbol(),
    }
}

pub(super) fn install_chain<G>(universe: &mut Universe<G>) -> ChainSymbols {
    let symbols = ChainSymbols {
        first: universe.intern("consumerchaina").expect("chain A").symbol(),
        second: universe.intern("consumerchainb").expect("chain B").symbol(),
        third: universe.intern("consumerchainc").expect("chain C").symbol(),
    };
    let mut words = Vec::with_capacity(32);
    append_definition(
        &mut words,
        symbols.first,
        &[
            Token::Cs(symbols.second),
            begin_group_token(),
            parameter(1),
            end_group_token(),
        ],
    );
    append_definition(
        &mut words,
        symbols.second,
        &[
            Token::Cs(symbols.third),
            begin_group_token(),
            parameter(1),
            end_group_token(),
        ],
    );
    append_definition(&mut words, symbols.third, &[parameter(1)]);

    let mut command = CommandState::default();
    let token_list = universe
        .allocate_token_list(&words)
        .expect("chain definition stream");
    let context = universe.command_context().expect("chain input context");
    command.push_everyjob(&context, token_list);
    drop(context);

    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut fuel = CommandFuelLedger::default();
    let operation = command.begin_attempt_operation();
    let mut context = universe
        .command_context()
        .expect("chain definition context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut scanned_definitions = Vec::new();
    for (target, expected) in [
        (
            symbols.first,
            vec![
                Token::Cs(symbols.second),
                begin_group_token(),
                parameter(1),
                end_group_token(),
            ],
        ),
        (
            symbols.second,
            vec![
                Token::Cs(symbols.third),
                begin_group_token(),
                parameter(1),
                end_group_token(),
            ],
        ),
        (symbols.third, vec![parameter(1)]),
    ] {
        let scanned = processor
            .scan_macro_definition(false, false)
            .expect("parameterized chain definition scan");
        assert_eq!(scanned.target, target);
        scanned_definitions.push((target, scanned.definition, expected));
    }
    drop(processor);
    command
        .commit_attempt_operation(operation)
        .expect("parameterized chain definition commit");
    for (target, definition, expected) in scanned_definitions {
        assert_definition_words(&context, definition, &[Token::Param(1)], &expected);
        context
            .assign_resolved_meaning(
                target,
                ResolvedMeaning::Macro {
                    flags: MeaningFlags::EMPTY,
                    definition,
                },
                AssignmentScope::Global,
            )
            .expect("parameterized chain meaning");
    }
    drop(context);
    symbols
}

pub(super) fn install_benchmark_catcodes<G>(universe: &mut Universe<G>) {
    let mut context = universe
        .command_context()
        .expect("consumer command context for catcodes");
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
            .expect("consumer fixture category code");
    }
}

pub(super) fn build_source_text(
    kind: WorkloadKind,
    operations: usize,
    text_chars: usize,
    body_words: usize,
) -> String {
    let mut unit = String::new();
    match kind {
        WorkloadKind::LongText => {
            unit.extend(std::iter::repeat_n(TEXT_CHARACTER, text_chars));
        }
        WorkloadKind::DefinitionBody => {
            unit.push_str(r"\consumerdefinition#1{");
            append_source_body(&mut unit, body_words);
            unit.push('}');
        }
        WorkloadKind::ParameterizedChain => unit.push_str(r"\consumerchaina{q}"),
        WorkloadKind::MixedPipeline => {
            unit.extend(std::iter::repeat_n(TEXT_CHARACTER, text_chars));
            unit.push_str(r"\consumerchaina{q}\consumerdefinition#1{");
            append_source_body(&mut unit, body_words);
            unit.push('}');
        }
    }
    unit.repeat(operations)
}

pub(super) fn build_stored_words<G>(
    universe: &mut Universe<G>,
    kind: WorkloadKind,
    operations: usize,
    text_chars: usize,
    body_words: usize,
) -> Vec<TokenWord> {
    let definition = universe
        .intern("consumerdefinition")
        .expect("stored definition target")
        .symbol();
    let body_control = universe
        .intern(BODY_CONTROL_NAME)
        .expect("stored definition body control")
        .symbol();
    let chain = [universe
        .intern("consumerchaina")
        .expect("stored chain A")
        .symbol()];
    let mut unit = Vec::new();
    match kind {
        WorkloadKind::LongText => {
            unit.extend(std::iter::repeat_n(char_word(TEXT_CHARACTER), text_chars));
        }
        WorkloadKind::DefinitionBody => {
            unit.push(TokenWord::pack(Token::Cs(definition)));
            unit.push(parameter_character());
            unit.push(other_word('1'));
            unit.push(begin_group());
            append_stored_body(&mut unit, body_control, body_words);
            unit.push(end_group());
        }
        WorkloadKind::ParameterizedChain => {
            unit.push(TokenWord::pack(Token::Cs(chain[0])));
            unit.push(begin_group());
            unit.push(char_word(CHAIN_RESULT_CHARACTER));
            unit.push(end_group());
        }
        WorkloadKind::MixedPipeline => {
            unit.extend(std::iter::repeat_n(char_word(TEXT_CHARACTER), text_chars));
            unit.push(TokenWord::pack(Token::Cs(chain[0])));
            unit.push(begin_group());
            unit.push(char_word(CHAIN_RESULT_CHARACTER));
            unit.push(end_group());
            unit.push(TokenWord::pack(Token::Cs(definition)));
            unit.push(parameter_character());
            unit.push(other_word('1'));
            unit.push(begin_group());
            append_stored_body(&mut unit, body_control, body_words);
            unit.push(end_group());
        }
    }
    unit.repeat(operations)
}

pub(super) fn open_source<G>(command: &mut CommandState<G>, source: &str) {
    let registered = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ))
        .expect("consumer source registration");
    command
        .open_registered_source(registered)
        .expect("consumer source opening");
}

pub(super) fn push_stored<G>(
    universe: &mut Universe<G>,
    command: &mut CommandState<G>,
    words: &[TokenWord],
) {
    let token_list = universe
        .allocate_token_list(words)
        .expect("consumer stored input allocation");
    let context = universe.command_context().expect("consumer stored context");
    command.push_everyjob(&context, token_list);
}

pub(super) fn assert_definition_words<G>(
    context: &tex_state::CommandContext<'_, G>,
    definition: tex_state::DefinitionRef<G>,
    parameters: &[Token],
    replacement: &[Token],
) {
    let view = context.definition(definition);
    let actual_parameters: Vec<_> = view
        .parameter_text()
        .iter()
        .map(|word| word.semantic_token())
        .collect();
    let actual_replacement: Vec<_> = view
        .replacement_text()
        .iter()
        .map(|word| word.semantic_token())
        .collect();
    assert_eq!(actual_parameters, parameters);
    assert_eq!(actual_replacement, replacement);
}

pub(super) fn validate_body<G>(
    context: &tex_state::CommandContext<'_, G>,
    definition: tex_state::DefinitionRef<G>,
    body_control: Symbol,
    words: usize,
) -> u64 {
    let view = context.definition(definition);
    let parameters: Vec<_> = view
        .parameter_text()
        .iter()
        .map(|word| word.semantic_token())
        .collect();
    assert_eq!(parameters, [Token::Param(1)]);
    let replacement = view.replacement_text();
    assert_eq!(replacement.len(), words);
    let mut checksum = 0_u64;
    for (index, word) in replacement.iter().enumerate() {
        assert_eq!(
            word.semantic_token(),
            body_token(body_control, index, words)
        );
        checksum = checksum.wrapping_add(body_token_value(index, words));
    }
    checksum
}

pub(super) fn with_universe<R>(
    run: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = tex_state::interner::InternerBudget::new(65_536, 65_536, 8 << 20)
        .expect("consumer interner budget");
    tex_state::with_universe(budget, run).expect("consumer universe")
}

fn append_definition(words: &mut Vec<TokenWord>, target: Symbol, replacement: &[Token]) {
    words.push(TokenWord::pack(Token::Cs(target)));
    words.push(parameter_character());
    words.push(other_word('1'));
    words.push(begin_group());
    // The scanner consumes TeX's source spelling (`#1`) and produces the
    // semantic `Token::Param(1)` in the stored definition.  The chain's
    // expected replacement vectors use the semantic token for readability,
    // so expand it back to the two raw input words at this fixture boundary.
    for token in replacement {
        match token {
            Token::Param(slot) => {
                assert!(*slot <= 9, "chain fixture parameter fits one digit");
                words.push(parameter_character());
                words.push(other_word(char::from(b'0' + *slot)));
            }
            token => words.push(TokenWord::pack(*token)),
        }
    }
    words.push(end_group());
}

fn parameter(slot: u8) -> Token {
    Token::Param(slot)
}

fn parameter_character() -> TokenWord {
    TokenWord::pack(Token::Char {
        ch: '#',
        cat: Catcode::Parameter,
    })
}

fn begin_group_token() -> Token {
    Token::Char {
        ch: '{',
        cat: Catcode::BeginGroup,
    }
}

fn end_group_token() -> Token {
    Token::Char {
        ch: '}',
        cat: Catcode::EndGroup,
    }
}

fn char_word(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Letter,
    })
}

fn other_word(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Other,
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
