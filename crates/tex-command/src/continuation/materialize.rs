use super::*;

pub(super) struct Materializer<'a> {
    owned: &'a OwnedCommandContinuation,
    universe: &'a mut Universe,
    source_ids: Vec<SourceId>,
    symbols: Vec<Symbol>,
    token_lists: Vec<Option<TokenListRef>>,
    origins: Vec<Option<OriginRef>>,
    origin_lists: Vec<Option<OriginListRef>>,
    macros: Vec<Option<MacroDefinitionRef>>,
}

impl<'a> Materializer<'a> {
    pub(super) fn new(owned: &'a OwnedCommandContinuation, universe: &'a mut Universe) -> Self {
        Self {
            owned,
            universe,
            source_ids: Vec::new(),
            symbols: Vec::new(),
            token_lists: vec![None; owned.token_lists.len()],
            origins: vec![None; owned.origins.len()],
            origin_lists: vec![None; owned.origin_lists.len()],
            macros: vec![None; owned.macros.len()],
        }
    }

    pub(super) fn finish(mut self) -> Result<CommandSummary, CommandContinuationError> {
        self.install_sources()?;
        self.install_symbols();
        let input = self.input(&self.owned.summary.input)?;
        let parameters = self.parameters(&self.owned.summary.parameters)?;
        Ok(CommandSummary {
            input,
            parameters,
            conditions: Self::conditions(&self.owned.summary.conditions),
            align_state: self.owned.summary.align_state,
            expansion: Self::expansion(&self.owned.summary.expansion),
            next_builder_identity: self.owned.summary.next_builder_identity,
        })
    }

    fn conditions(conditions: &OwnedConditionState) -> ConditionStack {
        ConditionStack {
            frames: conditions
                .frames
                .iter()
                .map(|frame| ConditionFrame {
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

    fn expansion(expansion: &OwnedExpansionState) -> ExpansionState {
        ExpansionState {
            cumulative_expansions: expansion.cumulative_expansions,
            next_resource_resolution: expansion.next_resource_resolution,
            pending_diagnostics: expansion.pending_diagnostics.clone(),
            observed_dependencies: expansion.observed_dependencies.clone(),
            semantic_barriers: expansion.semantic_barriers.clone(),
            profile: expansion.profile,
        }
    }

    fn generated_source_descriptor(source: &OwnedSourceRecipe) -> SourceDescriptor {
        match &source.descriptor {
            OwnedSourceDescriptor::World { .. } => {
                unreachable!("World descriptors are installed destination-locally")
            }
            OwnedSourceDescriptor::Generated {
                logical_path,
                bytes,
            } => logical_path.as_ref().map_or_else(
                || SourceDescriptor::generated(Arc::from(bytes.clone())),
                |path| SourceDescriptor::named_generated(path.clone(), Arc::from(bytes.clone())),
            ),
        }
    }

    fn install_sources(&mut self) -> Result<(), CommandContinuationError> {
        for source in &self.owned.sources {
            match &source.descriptor {
                OwnedSourceDescriptor::World {
                    path,
                    bytes,
                    modification_date,
                    origin,
                } => self
                    .universe
                    .install_detached_world_source(
                        source.id,
                        path.clone(),
                        Arc::from(bytes.clone()),
                        *modification_date,
                        *origin,
                    )
                    .map_err(CommandContinuationError::SourceMap)?,
                OwnedSourceDescriptor::Generated { .. } => self
                    .universe
                    .register_source(source.id, Self::generated_source_descriptor(source))
                    .map_err(CommandContinuationError::SourceMap)?,
            };
            self.source_ids.push(source.id);
        }
        Ok(())
    }

    fn install_symbols(&mut self) {
        self.symbols = self
            .owned
            .symbols
            .iter()
            .map(|symbol| match symbol.kind {
                ControlSequenceKind::ActiveCharacter => self
                    .universe
                    .intern_active_character(
                        symbol
                            .spelling
                            .chars()
                            .next()
                            .expect("validated active symbol"),
                    )
                    .symbol(),
                ControlSequenceKind::Internal => self
                    .universe
                    .intern_internal_control_sequence(&symbol.spelling)
                    .symbol(),
                ControlSequenceKind::Null
                | ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named => self.universe.intern(&symbol.spelling).symbol(),
            })
            .collect();
    }

    fn token(&self, token: &OwnedToken) -> Token {
        match token {
            OwnedToken::Character { ch, cat } => Token::Char { ch: *ch, cat: *cat },
            OwnedToken::Parameter(slot) => Token::Param(*slot),
            OwnedToken::Frozen(token) => Token::Frozen(*token),
            OwnedToken::ControlSequence(symbol) => Token::Cs(self.symbols[symbol.0]),
        }
    }

    fn token_list(&mut self, id: TokenListRecipeId) -> TokenListRef {
        if let Some(root) = &self.token_lists[id.0] {
            return root.clone();
        }
        let tokens = self.owned.token_lists[id.0]
            .iter()
            .map(|token| self.token(token))
            .collect::<Vec<_>>();
        let root = self.universe.intern_token_list_ref(&tokens);
        self.token_lists[id.0] = Some(root.clone());
        root
    }

    fn origin(&mut self, id: OriginRecipeId) -> OriginRef {
        if let Some(root) = &self.origins[id.0] {
            return root.clone();
        }
        let root = match self.owned.origins[id.0].clone() {
            OwnedOrigin::Unknown => OriginRef::unknown(),
            OwnedOrigin::Source {
                source,
                input_record,
                byte_offset,
                line,
                column,
            } => {
                let id = self.universe.source_origin_with_input_record(
                    self.source_ids[source.0],
                    input_record,
                    byte_offset,
                    line,
                    column,
                );
                self.universe
                    .origin_ref(id)
                    .unwrap_or_else(|| OriginRef::direct(id))
            }
            OwnedOrigin::SourceSpan { source, start, end } => self
                .universe
                .source_range_origin_ref(self.source_ids[source.0], start, end),
            OwnedOrigin::Synthetic(kind) => self.universe.synthetic_origin_ref(kind),
            OwnedOrigin::Synthesized { kind, parent } => {
                let parent = self.origin(parent);
                self.universe.synthesized_origin_ref(kind, parent)
            }
            OwnedOrigin::Inserted {
                kind,
                token,
                parent,
            } => {
                let token = self.token(&token);
                let parent = self.origin(parent);
                self.universe.inserted_origin_ref(kind, token, parent)
            }
            OwnedOrigin::ExpansionFrame {
                definition,
                detached_operand,
                invocation,
                definition_origin,
                parent,
            } => {
                let invocation = self.origin(invocation);
                let definition_origin = self.origin(definition_origin);
                let parent = self.origin(parent);
                let id = if let Some(definition) = definition {
                    let definition = self.macro_definition(definition);
                    self.universe.macro_invocation_origin(
                        definition.id(),
                        invocation.id(),
                        definition_origin.id(),
                        parent.id(),
                    )
                } else {
                    self.universe
                        .macro_invocation_origin_from_nonowning_operand(
                            detached_operand,
                            invocation.id(),
                            definition_origin.id(),
                            parent.id(),
                        )
                };
                self.universe
                    .materialize_origin_ref(id)
                    .expect("materialized invocation recipe is live")
            }
        };
        self.origins[id.0] = Some(root.clone());
        root
    }

    fn origin_list(&mut self, id: OriginListRecipeId) -> OriginListRef {
        if let Some(root) = &self.origin_lists[id.0] {
            return root.clone();
        }
        let origins = self.owned.origin_lists[id.0]
            .iter()
            .map(|origin| self.origin(*origin))
            .collect::<Vec<_>>();
        let root = self.universe.allocate_origin_list_ref(&origins);
        self.origin_lists[id.0] = Some(root.clone());
        root
    }

    fn macro_definition(&mut self, id: MacroRecipeId) -> MacroDefinitionRef {
        if let Some(root) = &self.macros[id.0] {
            return root.clone();
        }
        let recipe = self.owned.macros[id.0].clone();
        let parameters = self.token_list(recipe.parameters);
        let replacement = self.token_list(recipe.replacement);
        let root = self.universe.intern_macro(MacroMeaning::new(
            recipe.flags,
            parameters.id(),
            replacement.id(),
        ));
        self.macros[id.0] = Some(root.clone());
        let definition = self.origin(recipe.definition_origin);
        let parameter_origins = self.origin_list(recipe.parameter_origins);
        let replacement_origins = self.origin_list(recipe.replacement_origins);
        self.universe.set_macro_definition_provenance(
            root.id(),
            tex_state::macro_store::MacroDefinitionProvenance::new(
                definition,
                parameter_origins,
                replacement_origins,
            ),
        );
        root
    }

    fn word(&mut self, word: &OwnedWord) -> tex_state::token::RootedTracedTokenWord {
        tex_state::token::RootedTracedTokenWord::new(
            self.token(&word.token),
            self.origin(word.origin),
        )
    }

    fn registered_source(
        &mut self,
        source: &OwnedRegisteredSource,
    ) -> Result<RegisteredSource, CommandContinuationError> {
        let recipe = &self.owned.sources[source.source.0];
        let descriptor = match &recipe.descriptor {
            OwnedSourceDescriptor::World { .. } => self
                .universe
                .detached_source_descriptor(self.source_ids[source.source.0])
                .expect("installed World source has a destination-local descriptor"),
            // The source recipe owns the stable source-map registration. A
            // rebound editor cursor can legitimately own newer bytes under
            // that same coordinate identity, so rebuild its input descriptor
            // from the cursor backing rather than conflating the two roots.
            OwnedSourceDescriptor::Generated { logical_path, .. } => {
                logical_path.as_ref().map_or_else(
                    || SourceDescriptor::generated(Arc::from(source.bytes.clone())),
                    |path| {
                        SourceDescriptor::named_generated(
                            path.clone(),
                            Arc::from(source.bytes.clone()),
                        )
                    },
                )
            }
        };
        RegisteredSource::from_detached_parts(crate::input::DetachedRegisteredSourceParts {
            id: self.source_ids[source.source.0],
            kind: source.kind,
            mode: source.mode,
            bytes: Arc::from(source.bytes.clone()),
            name: source.name.clone().map(Arc::from),
            framing_name: source.framing_name.clone().map(Arc::from),
            framing: source.framing,
            descriptor,
        })
        .map_err(CommandContinuationError::SourceRegistration)
    }

    fn source_provenance(&self, source: &OwnedSourceProvenance) -> SourceProvenance {
        let id = self.source_ids[source.source.0];
        SourceProvenance::from_range_and_location(
            crate::SourceRange::new(id, source.start, source.end),
            crate::SourceLocation::new(id, source.location),
        )
    }

    fn source_range(&self, range: &OwnedSourceRange) -> crate::SourceRange {
        crate::SourceRange::new(self.source_ids[range.source.0], range.start, range.end)
    }

    fn source_line(&self, line: &OwnedSourceLineState) -> crate::input::SourceLineState {
        let content = self.source_range(&line.content);
        let terminator = self.source_range(&line.terminator);
        crate::input::SourceLineState {
            physical: crate::PhysicalLine::from_detached_parts(
                content.source(),
                line.number,
                content,
                terminator,
                line.terminator_kind,
            ),
            retained_end: line.retained_end,
            byte_cursor: line.byte_cursor,
            scalar_cursor: line.scalar_cursor,
            endline: line.endline,
            endline_delivered: line.endline_delivered,
            reduced_spellings: line
                .reduced_spellings
                .iter()
                .map(|(range, code)| crate::input::ReducedSourceSpelling {
                    range: self.source_range(range),
                    code: *code,
                })
                .collect(),
        }
    }

    fn backed_up(&mut self, word: &OwnedBackedUpToken) -> crate::input::RootedBackedUpToken {
        let spelling = self.word(&word.spelling);
        crate::input::RootedBackedUpToken::new(
            BackedUpToken {
                spelling: spelling.word(),
                source_provenance: word
                    .source_provenance
                    .as_ref()
                    .map(|source| self.source_provenance(source)),
            },
            spelling.into_parts().1,
        )
    }

    fn payload(&mut self, payload: &OwnedTokenPayload) -> TokenPayload {
        match payload {
            OwnedTokenPayload::Transient(words) => {
                TokenPayload::transient_rooted(words.iter().map(|word| self.word(word)))
            }
            OwnedTokenPayload::BackedUp(words) => {
                TokenPayload::backed_up_rooted(words.iter().map(|word| self.backed_up(word)))
            }
            OwnedTokenPayload::ArgumentRange { buffer, range } => TokenPayload::transient_rooted(
                buffer[range.start()..range.end()]
                    .iter()
                    .map(|word| self.word(word)),
            ),
        }
    }

    fn input(&mut self, input: &OwnedInputState) -> Result<InputState, CommandContinuationError> {
        Ok(InputState {
            levels: input
                .levels
                .iter()
                .map(|level| self.level(level))
                .collect::<Result<_, _>>()?,
            terminal_context_line: input.terminal_context_line.clone(),
            pending_sources: input
                .pending_sources
                .iter()
                .map(|(id, source)| Ok((*id, self.registered_source(source)?)))
                .collect::<Result<_, CommandContinuationError>>()?,
            next_level_identity: input.next_level_identity,
            next_source_identity: input.next_source_identity,
            force_eof: input.force_eof,
        })
    }

    fn level(&mut self, level: &OwnedInputLevel) -> Result<InputLevel, CommandContinuationError> {
        Ok(match level {
            OwnedInputLevel::Source(source) => {
                let backing = self.registered_source(&source.cursor.backing)?;
                let frame = crate::input::PackedInputFrame::source(source.identity.0, backing.id);
                InputLevel::Source(SourceLevel {
                    frame,
                    cursor: crate::input::SourceCursor {
                        backing,
                        line_backing: source
                            .cursor
                            .line_backing
                            .as_ref()
                            .map(|line| self.registered_source(line))
                            .transpose()?,
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
                    every_eof: source.every_eof.map(|(tokens, origins)| {
                        tex_state::TracedTokenList::new(
                            self.token_list(tokens),
                            self.origin_list(origins),
                        )
                    }),
                    open_depths: source.open_depths.as_ref().map(|depths| {
                        Box::new(SourceOpenDepths {
                            group_lineages: depths.group_lineages.clone().into_boxed_slice(),
                            conditional_identities: depths
                                .conditional_identities
                                .clone()
                                .into_boxed_slice(),
                        })
                    }),
                })
            }
            OwnedInputLevel::Tokens(cursor) => {
                let behavior = cursor.behavior.clone();
                let payload = self.payload(&cursor.payload);
                let mut frame = crate::input::packed_token_frame(
                    cursor.identity,
                    payload.frame_len(),
                    &behavior,
                    cursor.retirement,
                    &cursor.trace,
                );
                for _ in 0..cursor.index {
                    let _ = frame.advance();
                }
                InputLevel::Tokens(TokenCursor {
                    payload,
                    behavior,
                    retirement: cursor.retirement,
                    trace: cursor.trace.clone(),
                    frame,
                })
            }
        })
    }

    fn parameters(
        &mut self,
        parameters: &OwnedParameterState,
    ) -> Result<ParameterState, CommandContinuationError> {
        let mut state = ParameterState::default();
        for activation in &parameters.activations {
            let definition = self.macro_definition(activation.definition).id();
            let meaning = self.universe.macro_definition(definition).meaning();
            state.admit_macro(definition, meaning);
            let arguments = state.store_arguments(
                tex_state::token::RootedTracedTokenBuffer::new(
                    activation.arguments.iter().map(|word| self.word(word)),
                ),
                activation.ranges,
            );
            let invocation = self.origin(activation.invocation).id();
            state.restore_activation(
                activation.identity,
                self.symbols[activation.name.0],
                definition,
                arguments,
                invocation,
            );
        }
        state.next_activation_identity = parameters.next_activation_identity;
        Ok(state)
    }
}
