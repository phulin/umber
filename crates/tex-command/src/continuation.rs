//! Generation-independent ownership for retained command continuations.

use std::collections::{HashMap, HashSet};

use tex_state::Universe;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::interner::{ControlSequenceKind, Symbol};
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::provenance::OriginRecord;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::ParagraphInputTransaction;
use crate::input::{
    BackedUpToken, InputLevel, SharedBackedUpBuffer, SharedTokenBuffer, TokenPayload,
};
use crate::snapshot::CommandSummary;

#[derive(Clone, Debug)]
struct OwnedSymbol {
    kind: ControlSequenceKind,
    spelling: String,
}

#[derive(Clone, Debug)]
enum OwnedToken {
    Plain(Token),
    ControlSequence(OwnedSymbol),
}

#[derive(Clone, Debug)]
struct OwnedMacro {
    flags: tex_state::meaning::MeaningFlags,
    parameters: Vec<OwnedToken>,
    replacement: Vec<OwnedToken>,
    provenance: MacroDefinitionProvenance,
}

#[derive(Clone, Copy, Debug)]
enum OwnedOrigin {
    Record(OriginRecord),
    StableSource(tex_state::RootSpanId),
}

/// A command summary plus the complete arena-backed closure it reaches.
///
/// The copied summary is only a structural template. Every token-list,
/// origin-list, macro, origin and symbol reachable from it is represented by
/// owned semantic data and replaced before the summary is returned.
#[derive(Clone, Debug)]
pub struct OwnedCommandContinuation {
    summary: CommandSummary,
    token_lists: HashMap<TokenListId, Vec<OwnedToken>>,
    origin_lists: HashMap<OriginListId, Vec<OriginId>>,
    macros: HashMap<MacroDefinitionId, OwnedMacro>,
    origins: HashMap<OriginId, OwnedOrigin>,
    symbols: HashMap<Symbol, OwnedSymbol>,
    transactions: Vec<ParagraphInputTransaction>,
    allow_missing_origins: bool,
    accepted_resolvers: Vec<std::sync::Arc<tex_state::ParagraphOriginResolver>>,
}

impl OwnedCommandContinuation {
    /// Rebinds the sole retained paragraph transaction across an unchanged
    /// root prefix while preserving its detached arena closure.
    #[must_use]
    pub fn rebind_paragraph_unchanged_root_prefix(
        &mut self,
        old: &[u8],
        new: std::sync::Arc<[u8]>,
        unchanged_end: usize,
    ) -> bool {
        let [transaction] = self.transactions.as_slice() else {
            return false;
        };
        let Some(rebound) = transaction.rebind_unchanged_root_prefix(old, new, unchanged_end)
        else {
            return false;
        };
        self.transactions[0] = rebound;
        true
    }

    /// Rebinds the sole retained paragraph transaction around one root edit
    /// while preserving its detached arena closure.
    #[must_use]
    pub fn rebind_paragraph_edited_root(
        &mut self,
        old: &[u8],
        new: std::sync::Arc<[u8]>,
        edited: std::ops::Range<usize>,
    ) -> bool {
        let [transaction] = self.transactions.as_slice() else {
            return false;
        };
        let Some(rebound) = transaction.rebind_edited_root(old, new, edited) else {
            return false;
        };
        self.transactions[0] = rebound;
        true
    }

    #[must_use]
    pub fn detach(summary: &CommandSummary, universe: &Universe) -> Self {
        let mut owned = Self {
            summary: summary.clone(),
            token_lists: HashMap::new(),
            origin_lists: HashMap::new(),
            macros: HashMap::new(),
            origins: HashMap::new(),
            symbols: HashMap::new(),
            transactions: Vec::new(),
            allow_missing_origins: false,
            accepted_resolvers: Vec::new(),
        };
        owned.collect_summary(universe);
        owned
    }

    /// Detaches one checkpoint continuation and ordered paragraph endpoints
    /// into a single graph so shared stored identities materialize once.
    #[must_use]
    pub fn detach_with_paragraphs<'a>(
        summary: &CommandSummary,
        paragraphs: impl IntoIterator<
            Item = (
                &'a ParagraphInputTransaction,
                Option<std::sync::Arc<tex_state::ParagraphOriginResolver>>,
            ),
        >,
        universe: &Universe,
    ) -> Self {
        let paragraphs = paragraphs.into_iter().collect::<Vec<_>>();
        let mut owned = Self {
            summary: summary.clone(),
            token_lists: HashMap::new(),
            origin_lists: HashMap::new(),
            macros: HashMap::new(),
            origins: HashMap::new(),
            symbols: HashMap::new(),
            transactions: Vec::new(),
            allow_missing_origins: true,
            accepted_resolvers: paragraphs
                .iter()
                .filter_map(|(_, resolver)| resolver.clone())
                .collect(),
        };
        owned.collect_summary(universe);
        for (paragraph, _) in paragraphs {
            owned.collect_input(&paragraph.starting_input, universe);
            owned.collect_input(&paragraph.ending_input, universe);
            owned.collect_parameters(&paragraph.starting_parameters, universe);
            owned.collect_parameters(&paragraph.ending_parameters, universe);
            owned.transactions.push(paragraph.clone());
        }
        owned.allow_missing_origins = false;
        owned
    }

    /// Materializes the entire closure atomically into one destination.
    #[must_use]
    pub fn materialize_with_paragraphs(
        &self,
        universe: &mut Universe,
    ) -> (CommandSummary, Vec<ParagraphInputTransaction>) {
        let mut summary = self.summary.clone();
        let mut paragraphs = self.transactions.clone();
        let mut remap = Materializer::new(self, universe);
        remap.materialize_summary(&mut summary);
        for paragraph in &mut paragraphs {
            remap.materialize_input(&mut paragraph.starting_input);
            remap.materialize_input(&mut paragraph.ending_input);
            remap.materialize_parameters(&mut paragraph.starting_parameters);
            remap.materialize_parameters(&mut paragraph.ending_parameters);
        }
        (summary, paragraphs)
    }

    #[must_use]
    pub fn materialize(&self, universe: &mut Universe) -> CommandSummary {
        let mut summary = self.summary.clone();
        let mut remap = Materializer::new(self, universe);
        remap.materialize_summary(&mut summary);
        summary
    }

    fn collect_summary(&mut self, universe: &Universe) {
        let input = self.summary.input.clone();
        self.collect_input(&input, universe);
        let parameters = self.summary.parameters.clone();
        self.collect_parameters(&parameters, universe);
    }

    fn collect_parameters(
        &mut self,
        parameters: &crate::macro_call::ParameterState,
        universe: &Universe,
    ) {
        let activations = parameters.activations.clone();
        for activation in &activations {
            self.collect_symbol(activation.name, universe);
            self.collect_macro(activation.definition, universe);
            self.collect_words(activation.arguments.buffer.words(), universe);
            self.collect_origin(activation.invocation, universe);
        }
    }

    fn collect_input(&mut self, input: &crate::input::InputState, universe: &Universe) {
        let levels = input.levels.clone();
        for level in &levels {
            self.collect_level(level, universe);
        }
    }

    fn collect_level(&mut self, level: &InputLevel, universe: &Universe) {
        match level {
            InputLevel::Source(source) => {
                if let Some(list) = source.every_eof {
                    self.collect_token_list(list.token_list(), universe);
                    self.collect_origin_list(list.origin_list(), universe);
                }
            }
            InputLevel::Tokens(cursor) => match &cursor.payload {
                TokenPayload::Stored { tokens, origins } => {
                    self.collect_token_list(*tokens, universe);
                    self.collect_origin_list(*origins, universe);
                }
                TokenPayload::Transient(words) => self.collect_words(words.words(), universe),
                TokenPayload::InlineTransient(word) => self.collect_word(*word, universe),
                TokenPayload::BackedUp(words) => {
                    for word in words.words() {
                        self.collect_word(word.spelling, universe);
                    }
                }
                TokenPayload::InlineBackedUp(word) => self.collect_word(word.spelling, universe),
                TokenPayload::ArgumentRange { buffer, .. } => {
                    self.collect_words(buffer.words(), universe);
                }
            },
        }
    }

    fn collect_token_list(&mut self, id: TokenListId, universe: &Universe) {
        if self.token_lists.contains_key(&id) {
            return;
        }
        let tokens = universe.tokens(id).to_vec();
        self.token_lists.insert(id, Vec::new());
        let detached = tokens
            .into_iter()
            .map(|token| self.own_token(token, universe))
            .collect();
        self.token_lists.insert(id, detached);
    }

    fn collect_origin_list(&mut self, id: OriginListId, universe: &Universe) {
        if self.origin_lists.contains_key(&id) {
            return;
        }
        let origins = universe.origin_list(id).to_vec();
        self.origin_lists.insert(id, origins.clone());
        for origin in origins {
            self.collect_origin(origin, universe);
        }
    }

    fn collect_words(&mut self, words: &[TracedTokenWord], universe: &Universe) {
        for &word in words {
            self.collect_word(word, universe);
        }
    }

    fn collect_word(&mut self, word: TracedTokenWord, universe: &Universe) {
        let _ = self.own_token(word.semantic_token(), universe);
        self.collect_origin(word.origin(), universe);
    }

    fn collect_symbol(&mut self, symbol: Symbol, universe: &Universe) {
        self.symbols.entry(symbol).or_insert_with(|| OwnedSymbol {
            kind: universe.control_sequence_kind(symbol),
            spelling: universe.resolve(symbol).to_owned(),
        });
    }

    fn own_token(&mut self, token: Token, universe: &Universe) -> OwnedToken {
        match token {
            Token::Cs(symbol) => {
                self.collect_symbol(symbol, universe);
                OwnedToken::ControlSequence(self.symbols[&symbol].clone())
            }
            token => OwnedToken::Plain(token),
        }
    }

    fn collect_macro(&mut self, id: MacroDefinitionId, universe: &Universe) {
        if self.macros.contains_key(&id) {
            return;
        }
        let meaning = universe.macro_definition(id);
        let provenance = universe.macro_definition_provenance(id);
        self.macros.insert(
            id,
            OwnedMacro {
                flags: meaning.flags(),
                parameters: Vec::new(),
                replacement: Vec::new(),
                provenance,
            },
        );
        let parameters = universe.tokens(meaning.parameter_text()).to_vec();
        let replacement = universe.tokens(meaning.replacement_text()).to_vec();
        let parameters = parameters
            .into_iter()
            .map(|token| self.own_token(token, universe))
            .collect();
        let replacement = replacement
            .into_iter()
            .map(|token| self.own_token(token, universe))
            .collect();
        self.macros
            .get_mut(&id)
            .expect("macro placeholder")
            .parameters = parameters;
        self.macros
            .get_mut(&id)
            .expect("macro placeholder")
            .replacement = replacement;
        self.collect_origin(provenance.definition_origin(), universe);
        self.collect_origin_list(provenance.parameter_origins(), universe);
        self.collect_origin_list(provenance.replacement_origins(), universe);
    }

    fn collect_origin(&mut self, id: OriginId, universe: &Universe) {
        if id == OriginId::UNKNOWN || self.origins.contains_key(&id) {
            return;
        }
        let live_record = universe.origin_if_live(id);
        let record = live_record.or_else(|| {
            self.accepted_resolvers
                .iter()
                .find_map(|resolver| resolver.origin_record(id))
        });
        let Some(record) = record else {
            if let Some(span) = self
                .accepted_resolvers
                .iter()
                .find_map(|resolver| resolver.stable_span(id))
            {
                self.origins.insert(id, OwnedOrigin::StableSource(span));
            } else {
                assert!(
                    self.allow_missing_origins,
                    "command continuation origin is not live"
                );
                self.origins
                    .insert(id, OwnedOrigin::Record(OriginRecord::UnknownBootstrap));
            }
            return;
        };
        if live_record.is_none()
            && matches!(
                record,
                OriginRecord::Source(_) | OriginRecord::SourceSpan(_)
            )
            && let Some(span) = self
                .accepted_resolvers
                .iter()
                .find_map(|resolver| resolver.stable_span(id))
        {
            self.origins.insert(id, OwnedOrigin::StableSource(span));
            return;
        }
        self.origins.insert(id, OwnedOrigin::Record(record));
        match record {
            OriginRecord::MacroInvocation(origin) => {
                self.collect_macro(origin.definition(), universe);
                self.collect_origin(origin.invocation(), universe);
                self.collect_origin(origin.definition_origin(), universe);
                self.collect_origin(origin.parent_invocation(), universe);
            }
            OriginRecord::Inserted(origin) => {
                let _ = self.own_token(origin.token(), universe);
                self.collect_origin(origin.parent(), universe);
            }
            OriginRecord::Synthesized(origin) => self.collect_origin(origin.parent(), universe),
            OriginRecord::UnknownBootstrap
            | OriginRecord::Source(_)
            | OriginRecord::SourceSpan(_)
            | OriginRecord::Synthetic(_) => {}
        }
    }
}

struct Materializer<'a> {
    owned: &'a OwnedCommandContinuation,
    universe: &'a mut Universe,
    tokens: HashMap<TokenListId, TokenListId>,
    origin_lists: HashMap<OriginListId, OriginListId>,
    macros: HashMap<MacroDefinitionId, MacroDefinitionId>,
    origins: HashMap<OriginId, OriginId>,
    macro_provenance_done: HashSet<MacroDefinitionId>,
}

impl<'a> Materializer<'a> {
    fn new(owned: &'a OwnedCommandContinuation, universe: &'a mut Universe) -> Self {
        Self {
            owned,
            universe,
            tokens: HashMap::new(),
            origin_lists: HashMap::new(),
            macros: HashMap::new(),
            origins: HashMap::new(),
            macro_provenance_done: HashSet::new(),
        }
    }

    fn token(&mut self, token: &OwnedToken) -> Token {
        match token {
            OwnedToken::Plain(token) => *token,
            OwnedToken::ControlSequence(symbol) => Token::Cs(match symbol.kind {
                ControlSequenceKind::ActiveCharacter => self
                    .universe
                    .intern_active_character(
                        symbol.spelling.chars().next().expect("active spelling"),
                    )
                    .symbol(),
                ControlSequenceKind::Null
                | ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named => self.universe.intern(&symbol.spelling).symbol(),
                ControlSequenceKind::Internal => self
                    .universe
                    .intern_internal_control_sequence(&symbol.spelling)
                    .symbol(),
            }),
        }
    }

    fn token_list(&mut self, old: TokenListId) -> TokenListId {
        if let Some(&id) = self.tokens.get(&old) {
            return id;
        }
        let owned = self
            .owned
            .token_lists
            .get(&old)
            .expect("detached token list")
            .clone();
        let tokens = owned
            .iter()
            .map(|token| self.token(token))
            .collect::<Vec<_>>();
        let id = self.universe.intern_token_list(&tokens);
        self.tokens.insert(old, id);
        id
    }

    fn macro_id(&mut self, old: MacroDefinitionId) -> MacroDefinitionId {
        if let Some(&id) = self.macros.get(&old) {
            return id;
        }
        let owned = self.owned.macros.get(&old).expect("detached macro").clone();
        let parameters = owned
            .parameters
            .iter()
            .map(|token| self.token(token))
            .collect::<Vec<_>>();
        let replacement = owned
            .replacement
            .iter()
            .map(|token| self.token(token))
            .collect::<Vec<_>>();
        let parameters = self.universe.intern_token_list(&parameters);
        let replacement = self.universe.intern_token_list(&replacement);
        let id =
            self.universe
                .intern_macro(MacroMeaning::new(owned.flags, parameters, replacement));
        self.macros.insert(old, id);
        id
    }

    fn finish_macro_provenance(&mut self, old: MacroDefinitionId) {
        if !self.macro_provenance_done.insert(old) {
            return;
        }
        let id = self.macro_id(old);
        let provenance = self.owned.macros[&old].provenance;
        let definition = self.origin(provenance.definition_origin());
        let parameters = self.origin_list(provenance.parameter_origins());
        let replacement = self.origin_list(provenance.replacement_origins());
        self.universe.set_macro_definition_provenance(
            id,
            MacroDefinitionProvenance::new(definition, parameters, replacement),
        );
    }

    fn origin(&mut self, old: OriginId) -> OriginId {
        if old == OriginId::UNKNOWN {
            return old;
        }
        if let Some(&id) = self.origins.get(&old) {
            return id;
        }
        let record = self.owned.origins[&old];
        let id = match record {
            OwnedOrigin::StableSource(span) => self
                .universe
                .origin_for_root_span(span)
                .expect("detached stable continuation source exists in the rebound layout"),
            OwnedOrigin::Record(record) => match record {
                OriginRecord::UnknownBootstrap => self.universe.bootstrap_origin(),
                OriginRecord::Source(source) => self.universe.source_origin_with_input_record(
                    source.source(),
                    source.input_record(),
                    source.byte_offset(),
                    source.line(),
                    source.column(),
                ),
                OriginRecord::SourceSpan(span) => self.universe.source_span_origin(span),
                OriginRecord::Synthetic(origin) => self.universe.synthetic_origin(origin.kind()),
                OriginRecord::Synthesized(origin) => {
                    let parent = self.origin(origin.parent());
                    self.universe.synthesized_origin(origin.kind(), parent)
                }
                OriginRecord::Inserted(origin) => {
                    let token = self.token(&self.owned_token(origin.token()));
                    let parent = self.origin(origin.parent());
                    self.universe.inserted_origin(origin.kind(), token, parent)
                }
                OriginRecord::MacroInvocation(origin) => {
                    let definition = self.macro_id(origin.definition());
                    let invocation = self.origin(origin.invocation());
                    let definition_origin = self.origin(origin.definition_origin());
                    let parent = self.origin(origin.parent_invocation());
                    self.universe.macro_invocation_origin(
                        definition,
                        invocation,
                        definition_origin,
                        parent,
                    )
                }
            },
        };
        self.origins.insert(old, id);
        id
    }

    fn owned_token(&self, token: Token) -> OwnedToken {
        match token {
            Token::Cs(symbol) => OwnedToken::ControlSequence(self.owned.symbols[&symbol].clone()),
            token => OwnedToken::Plain(token),
        }
    }

    fn origin_list(&mut self, old: OriginListId) -> OriginListId {
        if let Some(&id) = self.origin_lists.get(&old) {
            return id;
        }
        let origins = self
            .owned
            .origin_lists
            .get(&old)
            .expect("detached origin list")
            .clone()
            .into_iter()
            .map(|origin| self.origin(origin))
            .collect::<Vec<_>>();
        let id = self.universe.allocate_origin_list(&origins);
        self.origin_lists.insert(old, id);
        id
    }

    fn word(&mut self, word: TracedTokenWord) -> TracedTokenWord {
        let token = self.token(&self.owned_token(word.semantic_token()));
        TracedTokenWord::pack(token, self.origin(word.origin()))
    }

    fn materialize_summary(&mut self, summary: &mut CommandSummary) {
        self.materialize_input(&mut summary.input);
        self.materialize_parameters(&mut summary.parameters);
    }

    fn materialize_parameters(&mut self, parameters: &mut crate::macro_call::ParameterState) {
        for activation in &mut parameters.activations {
            activation.name = match self.token(&OwnedToken::ControlSequence(
                self.owned.symbols[&activation.name].clone(),
            )) {
                Token::Cs(symbol) => symbol,
                _ => unreachable!(),
            };
            let old = activation.definition;
            activation.definition = self.macro_id(old);
            activation.arguments.buffer = SharedTokenBuffer::new(
                activation
                    .arguments
                    .buffer
                    .words()
                    .iter()
                    .copied()
                    .map(|word| self.word(word))
                    .collect::<Vec<_>>(),
            );
            activation.invocation = self.origin(activation.invocation);
            self.finish_macro_provenance(old);
        }
    }

    fn materialize_input(&mut self, input: &mut crate::input::InputState) {
        for level in &mut input.levels {
            match level {
                InputLevel::Source(source) => {
                    if let Some(list) = &mut source.every_eof {
                        *list = tex_state::TracedTokenList::new(
                            self.token_list(list.token_list()),
                            self.origin_list(list.origin_list()),
                        );
                    }
                }
                InputLevel::Tokens(cursor) => match &mut cursor.payload {
                    TokenPayload::Stored { tokens, origins } => {
                        *tokens = self.token_list(*tokens);
                        *origins = self.origin_list(*origins);
                    }
                    TokenPayload::Transient(words) => {
                        *words = SharedTokenBuffer::new(
                            words
                                .words()
                                .iter()
                                .copied()
                                .map(|word| self.word(word))
                                .collect::<Vec<_>>(),
                        )
                    }
                    TokenPayload::InlineTransient(word) => *word = self.word(*word),
                    TokenPayload::BackedUp(words) => {
                        *words = SharedBackedUpBuffer::new(
                            words
                                .words()
                                .iter()
                                .map(|word| BackedUpToken {
                                    spelling: self.word(word.spelling),
                                    source_provenance: word.source_provenance,
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                    TokenPayload::InlineBackedUp(word) => {
                        word.spelling = self.word(word.spelling);
                    }
                    TokenPayload::ArgumentRange { buffer, .. } => {
                        *buffer = SharedTokenBuffer::new(
                            buffer
                                .words()
                                .iter()
                                .copied()
                                .map(|word| self.word(word))
                                .collect::<Vec<_>>(),
                        )
                    }
                },
            }
        }
    }
}
