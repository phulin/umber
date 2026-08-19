use super::*;

pub(super) struct Detacher<'a> {
    universe: &'a Universe,
    sources: Vec<OwnedSourceRecipe>,
    source_ids: HashMap<SourceId, SourceRecipeId>,
    symbols: Vec<OwnedSymbol>,
    symbol_ids: HashMap<Symbol, SymbolRecipeId>,
    token_lists: Vec<Vec<OwnedToken>>,
    token_list_ids: HashMap<TokenListId, TokenListRecipeId>,
    origins: Vec<OwnedOrigin>,
    origin_ids: HashMap<OriginId, OriginRecipeId>,
    origin_lists: Vec<Vec<OriginRecipeId>>,
    origin_list_ids: HashMap<OriginListId, OriginListRecipeId>,
    macros: Vec<OwnedMacro>,
    macro_ids: HashMap<MacroDefinitionId, MacroRecipeId>,
    macro_operands: HashMap<u64, MacroRecipeId>,
}

impl<'a> Detacher<'a> {
    pub(super) fn new(universe: &'a Universe) -> Self {
        let mut this = Self {
            universe,
            sources: Vec::new(),
            source_ids: HashMap::new(),
            symbols: Vec::new(),
            symbol_ids: HashMap::new(),
            token_lists: Vec::new(),
            token_list_ids: HashMap::new(),
            origins: vec![OwnedOrigin::Unknown],
            origin_ids: HashMap::from([(OriginId::UNKNOWN, OriginRecipeId(0))]),
            origin_lists: vec![Vec::new()],
            origin_list_ids: HashMap::from([(OriginListId::EMPTY, OriginListRecipeId(0))]),
            macros: Vec::new(),
            macro_ids: HashMap::new(),
            macro_operands: HashMap::new(),
        };
        let empty = universe.token_list_ref(TokenListId::EMPTY);
        this.token_list(&empty);
        this
    }

    pub(super) fn finish(mut self, summary: &CommandSummary) -> OwnedCommandContinuation {
        let input = self.input(&summary.input, &summary.parameters);
        let parameters = self.parameters(&summary.parameters);
        OwnedCommandContinuation {
            summary: OwnedCommandSummary {
                input,
                parameters,
                conditions: Self::conditions(&summary.conditions),
                align_state: summary.align_state,
                expansion: Self::expansion(&summary.expansion),
                next_builder_identity: summary.next_builder_identity,
            },
            sources: self.sources,
            symbols: self.symbols,
            token_lists: self.token_lists,
            origins: self.origins,
            origin_lists: self.origin_lists,
            macros: self.macros,
        }
    }

    fn conditions(conditions: &ConditionStack) -> OwnedConditionState {
        OwnedConditionState {
            frames: conditions
                .frames
                .iter()
                .map(|frame| OwnedConditionFrame {
                    identity: frame.identity,
                    kind: frame.kind,
                    limit: frame.limit,
                    source_line: frame.source_line,
                    inverted: frame.inverted,
                })
                .collect(),
            next_identity: conditions.next_identity,
        }
    }

    fn expansion(expansion: &ExpansionState) -> OwnedExpansionState {
        OwnedExpansionState {
            cumulative_expansions: expansion.cumulative_expansions,
            next_resource_resolution: expansion.next_resource_resolution,
            pending_diagnostics: expansion.pending_diagnostics.clone(),
            observed_dependencies: expansion.observed_dependencies.clone(),
            semantic_barriers: expansion.semantic_barriers.clone(),
            profile: expansion.profile,
        }
    }

    fn source_descriptor(&mut self, id: SourceId, descriptor: SourceDescriptor) -> SourceRecipeId {
        if let Some(recipe) = self.source_ids.get(&id) {
            return *recipe;
        }
        let descriptor = match descriptor {
            SourceDescriptor::World {
                input_record,
                byte_len: _,
            } => {
                let (path, bytes, modification_date, origin) = self
                    .universe
                    .detached_world_input(input_record)
                    .expect("continuation source owns its World input backing");
                OwnedSourceDescriptor::World {
                    path,
                    bytes,
                    modification_date,
                    origin,
                }
            }
            SourceDescriptor::Generated(source) => OwnedSourceDescriptor::Generated {
                logical_path: source.logical_path().map(str::to_owned),
                bytes: source.bytes().to_vec(),
            },
        };
        let recipe = SourceRecipeId(self.sources.len());
        self.sources.push(OwnedSourceRecipe { id, descriptor });
        self.source_ids.insert(id, recipe);
        recipe
    }

    fn source_id(&mut self, id: SourceId) -> SourceRecipeId {
        if let Some(recipe) = self.source_ids.get(&id) {
            return *recipe;
        }
        let descriptor = self
            .universe
            .detached_source_descriptor(id)
            .expect("continuation origin owns its source registration");
        self.source_descriptor(id, descriptor)
    }

    fn registered_source(&mut self, source: &RegisteredSource) -> OwnedRegisteredSource {
        // An editor-root cursor may have been rebound onto newer revision
        // bytes while its stable source-map coordinate registration remains
        // the one retained by the accepted substrate. Keep those two recipes
        // distinct: the source recipe describes the structural registration,
        // while this value's bytes describe the live cursor backing.
        let live_descriptor = source.source_descriptor();
        let retained_descriptor = self.universe.detached_source_descriptor(source.id);
        let rebound_registration = retained_descriptor
            .as_ref()
            .is_some_and(|descriptor| descriptor != &live_descriptor);
        let source_recipe = if let Some(descriptor) = retained_descriptor {
            self.source_descriptor(source.id, descriptor)
        } else {
            // JobStart precedes first delivery and therefore may not have
            // published the root registration yet.
            self.source_descriptor(source.id, source.source_descriptor())
        };
        OwnedRegisteredSource {
            source: source_recipe,
            rebound_registration,
            kind: source.kind,
            mode: source.mode,
            bytes: source.bytes.to_vec(),
            name: source.name.as_deref().map(str::to_owned),
            framing_name: source.framing_name.as_deref().map(str::to_owned),
            framing: source.framing,
        }
    }

    fn symbol(&mut self, symbol: Symbol) -> SymbolRecipeId {
        if let Some(recipe) = self.symbol_ids.get(&symbol) {
            return *recipe;
        }
        let recipe = SymbolRecipeId(self.symbols.len());
        self.symbols.push(OwnedSymbol {
            kind: self.universe.control_sequence_kind(symbol),
            spelling: self.universe.resolve(symbol).to_owned(),
        });
        self.symbol_ids.insert(symbol, recipe);
        recipe
    }

    fn token(&mut self, token: Token) -> OwnedToken {
        match token {
            Token::Cs(symbol) => OwnedToken::ControlSequence(self.symbol(symbol)),
            Token::Char { ch, cat } => OwnedToken::Character { ch, cat },
            Token::Param(slot) => OwnedToken::Parameter(slot),
            Token::Frozen(token) => OwnedToken::Frozen(token),
        }
    }

    fn token_list(&mut self, root: &TokenListRef) -> TokenListRecipeId {
        if let Some(recipe) = self.token_list_ids.get(&root.id()) {
            return *recipe;
        }
        let recipe = TokenListRecipeId(self.token_lists.len());
        self.token_list_ids.insert(root.id(), recipe);
        self.token_lists.push(Vec::new());
        let tokens = root
            .tokens()
            .iter()
            .map(|token| self.token(*token))
            .collect();
        self.token_lists[recipe.0] = tokens;
        recipe
    }

    fn origin_list(&mut self, root: &OriginListRef) -> OriginListRecipeId {
        if let Some(recipe) = self.origin_list_ids.get(&root.id()) {
            return *recipe;
        }
        let recipe = OriginListRecipeId(self.origin_lists.len());
        self.origin_list_ids.insert(root.id(), recipe);
        self.origin_lists.push(Vec::new());
        let origins = root
            .roots()
            .map(|origin| self.origin_ref(&origin))
            .collect();
        self.origin_lists[recipe.0] = origins;
        recipe
    }

    fn origin_words(&mut self, origins: impl IntoIterator<Item = OriginId>) -> OriginListRecipeId {
        let origins = origins
            .into_iter()
            .map(|origin| self.origin_id(origin))
            .collect::<Vec<_>>();
        if origins.is_empty() {
            return OriginListRecipeId(0);
        }
        let recipe = OriginListRecipeId(self.origin_lists.len());
        self.origin_lists.push(origins);
        recipe
    }

    fn origin_id(&mut self, id: OriginId) -> OriginRecipeId {
        if let Some(recipe) = self.origin_ids.get(&id) {
            return *recipe;
        }
        let root = self
            .universe
            .origin_ref(id)
            .unwrap_or_else(|| OriginRef::direct(id));
        self.origin_ref(&root)
    }

    fn origin_ref(&mut self, root: &OriginRef) -> OriginRecipeId {
        let id = root.id();
        if let Some(recipe) = self.origin_ids.get(&id) {
            return *recipe;
        }
        let recipe = OriginRecipeId(self.origins.len());
        self.origin_ids.insert(id, recipe);
        self.origins.push(OwnedOrigin::Unknown);
        let record = root
            .record()
            .or_else(|| self.universe.origin_if_live(id))
            .expect("continuation origin is structurally live");
        let owned = match record {
            OriginRecord::UnknownBootstrap => OwnedOrigin::Unknown,
            OriginRecord::Source(source) => OwnedOrigin::Source {
                source: self.source_id(source.source()),
                input_record: source.input_record(),
                byte_offset: source.byte_offset(),
                line: source.line(),
                column: source.column(),
            },
            OriginRecord::SourceSpan(span) => {
                let (source, start, end) = self
                    .universe
                    .detached_source_span(span)
                    .expect("continuation source span owns its registration");
                OwnedOrigin::SourceSpan {
                    source: self.source_id(source),
                    start,
                    end,
                }
            }
            OriginRecord::Synthetic(origin) => OwnedOrigin::Synthetic(origin.kind()),
            OriginRecord::Synthesized(origin) => OwnedOrigin::Synthesized {
                kind: origin.kind(),
                parent: self.origin_id(origin.parent()),
            },
            OriginRecord::Inserted(origin) => OwnedOrigin::Inserted {
                kind: origin.kind(),
                token: self.token(origin.token()),
                parent: self.origin_id(origin.parent()),
            },
            OriginRecord::MacroInvocation(origin) => OwnedOrigin::ExpansionFrame {
                definition: self
                    .macro_operands
                    .get(&origin.definition_operand())
                    .copied(),
                detached_operand: origin.definition_operand(),
                invocation: self.origin_id(origin.invocation()),
                definition_origin: self.origin_id(origin.definition_origin()),
                parent: self.origin_id(origin.parent_invocation()),
            },
        };
        self.origins[recipe.0] = owned;
        recipe
    }

    fn word(&mut self, word: tex_state::token::RootedTracedTokenWord) -> OwnedWord {
        OwnedWord {
            token: self.token(word.word().semantic_token()),
            origin: self.origin_ref(word.origin_ref()),
        }
    }

    fn source_provenance(&mut self, provenance: SourceProvenance) -> OwnedSourceProvenance {
        let range = provenance.range();
        OwnedSourceProvenance {
            source: self.source_id(range.source()),
            start: range.start(),
            end: range.end(),
            location: provenance.location().byte(),
        }
    }

    fn source_range(&mut self, range: crate::SourceRange) -> OwnedSourceRange {
        OwnedSourceRange {
            source: self.source_id(range.source()),
            start: range.start(),
            end: range.end(),
        }
    }

    fn source_line(&mut self, line: &crate::input::SourceLineState) -> OwnedSourceLineState {
        OwnedSourceLineState {
            number: line.physical.number(),
            content: self.source_range(line.physical.content_range()),
            terminator: self.source_range(line.physical.terminator_range()),
            terminator_kind: line.physical.terminator(),
            retained_end: line.retained_end,
            byte_cursor: line.byte_cursor,
            scalar_cursor: line.scalar_cursor,
            endline: line.endline,
            endline_delivered: line.endline_delivered,
            reduced_spellings: line
                .reduced_spellings
                .iter()
                .map(|spelling| (self.source_range(spelling.range), spelling.code))
                .collect(),
        }
    }

    fn macro_definition(
        &mut self,
        definition: MacroDefinitionId,
        parameters: &ParameterState,
    ) -> MacroRecipeId {
        if let Some(recipe) = self.macro_ids.get(&definition) {
            return *recipe;
        }
        let recipe = MacroRecipeId(self.macros.len());
        self.macro_ids.insert(definition, recipe);
        let owner = parameters.macro_owner(definition);
        self.macro_operands.insert(
            owner
                .observation_operand(definition)
                .expect("admitted macro has an observation operand") as u64,
            recipe,
        );
        self.macros.push(OwnedMacro {
            flags: tex_state::meaning::MeaningFlags::from_bits(0),
            parameters: TokenListRecipeId(0),
            replacement: TokenListRecipeId(0),
            definition_origin: OriginRecipeId(0),
            parameter_origins: OriginListRecipeId(0),
            replacement_origins: OriginListRecipeId(0),
        });
        let meaning = owner
            .meaning(definition)
            .expect("admitted macro has a packed meaning");
        let parameters = self.token_list(&self.universe.token_list_ref(meaning.parameter_text()));
        let replacement =
            self.token_list(&self.universe.token_list_ref(meaning.replacement_text()));
        let definition_origin = self.origin_id(
            owner
                .definition_origin(definition)
                .unwrap_or(OriginId::UNKNOWN),
        );
        let parameter_origins = self.origin_words(
            (0..owner.parameter_len(definition).unwrap_or(0)).filter_map(|index| {
                owner
                    .parameter_traced_word(definition, index)
                    .map(|word| word.origin())
            }),
        );
        let replacement_origins = self.origin_words(
            (0..owner.replacement_len(definition).unwrap_or(0)).filter_map(|index| {
                owner
                    .replacement_traced_word(definition, index)
                    .map(|word| word.origin())
            }),
        );
        self.macros[recipe.0] = OwnedMacro {
            flags: meaning.flags(),
            parameters,
            replacement,
            definition_origin,
            parameter_origins,
            replacement_origins,
        };
        recipe
    }

    fn input(&mut self, input: &InputState, parameters: &ParameterState) -> OwnedInputState {
        OwnedInputState {
            levels: input
                .levels
                .iter()
                .map(|level| self.level(level, parameters))
                .collect(),
            terminal_context_line: input.terminal_context_line.clone(),
            pending_sources: input
                .pending_sources
                .iter()
                .map(|(id, source)| (*id, self.registered_source(source)))
                .collect(),
            next_level_identity: input.next_level_identity,
            next_source_identity: input.next_source_identity,
            force_eof: input.force_eof,
        }
    }

    fn level(&mut self, level: &InputLevel, parameters: &ParameterState) -> OwnedInputLevel {
        match level {
            InputLevel::Source(source) => OwnedInputLevel::Source(Box::new(OwnedSourceLevel {
                identity: source.identity(),
                cursor: OwnedSourceCursor {
                    backing: self.registered_source(&source.cursor.backing),
                    line_backing: source
                        .cursor
                        .line_backing
                        .as_ref()
                        .map(|source| self.registered_source(source)),
                    pending_acquired_line: source.cursor.pending_acquired_line,
                    next_physical_offset: source.cursor.next_physical_offset,
                    next_line_number: source.cursor.next_line_number,
                    line: source
                        .cursor
                        .line
                        .as_ref()
                        .map(|line| self.source_line(line)),
                    lexer_state: source.cursor.lexer_state,
                    end_after_line: source.cursor.end_after_line,
                },
                name_class: source.name_class,
                retirement: source.retirement,
                every_eof: source.every_eof.as_ref().map(|list| {
                    (
                        self.token_list(list.token_ref()),
                        self.origin_list(list.origin_ref()),
                    )
                }),
                open_depths: source
                    .open_depths
                    .as_deref()
                    .map(|depths| OwnedSourceOpenDepths {
                        group_lineages: depths.group_lineages.to_vec(),
                        conditional_identities: depths.conditional_identities.to_vec(),
                    }),
            })),
            InputLevel::Tokens(cursor) => OwnedInputLevel::Tokens(OwnedTokenCursor {
                payload: self.payload(&cursor.payload, parameters),
                behavior: cursor.behavior.clone(),
                retirement: cursor.retirement,
                trace: cursor.trace.clone(),
                index: cursor.position(),
                identity: cursor.identity(),
            }),
        }
    }

    fn payload(
        &mut self,
        payload: &TokenPayload,
        parameters: &ParameterState,
    ) -> OwnedTokenPayload {
        match payload {
            TokenPayload::Packed(chunk) if chunk.is_backed_up() => OwnedTokenPayload::BackedUp(
                chunk
                    .rooted_words()
                    .zip(chunk.source_provenance())
                    .map(|(word, source_provenance)| {
                        let (spelling, root) = word.into_parts();
                        self.backed_up(crate::input::RootedBackedUpToken::new(
                            crate::input::BackedUpToken {
                                spelling,
                                source_provenance: *source_provenance,
                            },
                            root,
                        ))
                    })
                    .collect(),
            ),
            TokenPayload::Packed(chunk) => OwnedTokenPayload::Transient(
                chunk.rooted_words().map(|word| self.word(word)).collect(),
            ),
            TokenPayload::MacroReplacement {
                admitted,
                definition,
                len,
            } => OwnedTokenPayload::Transient(
                (0..*len as usize)
                    .filter_map(|index| {
                        parameters
                            .admitted_macro(*admitted)
                            .replacement_word(*definition, index)
                    })
                    .map(|word| self.word(word))
                    .collect(),
            ),
            TokenPayload::ArgumentRange { arguments, range } => OwnedTokenPayload::ArgumentRange {
                buffer: parameters
                    .argument_rooted_words(*arguments)
                    .map(|word| self.word(word))
                    .collect(),
                range: *range,
            },
        }
    }

    fn backed_up(&mut self, word: crate::input::RootedBackedUpToken) -> OwnedBackedUpToken {
        let (word, root) = word.into_parts();
        OwnedBackedUpToken {
            spelling: self.word(tex_state::token::RootedTracedTokenWord::from_word(
                word.spelling,
                root,
            )),
            source_provenance: word
                .source_provenance
                .map(|source| self.source_provenance(source)),
        }
    }

    fn parameters(&mut self, parameters: &ParameterState) -> OwnedParameterState {
        OwnedParameterState {
            activations: parameters
                .activations
                .iter()
                .map(|activation| OwnedActivation {
                    identity: activation.identity,
                    name: self.symbol(activation.name),
                    definition: self.macro_definition(activation.definition, parameters),
                    arguments: parameters
                        .argument_rooted_words(activation.arguments)
                        .map(|word| self.word(word))
                        .collect(),
                    ranges: parameters.argument_ranges(activation.arguments),
                    invocation: self.origin_id(activation.invocation),
                })
                .collect(),
            next_activation_identity: parameters.next_activation_identity,
        }
    }
}
