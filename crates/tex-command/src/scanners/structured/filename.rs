use super::*;

impl<G> CommandProcessor<'_, '_, G> {
    /// TeX's `scan_file_name`, returning a typed boundary instead of an input
    /// cursor or a backed-up raw command.
    fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        self.command.begin_file_name()?;
        let result = self.scan_file_name_inner();
        self.command.end_file_name();
        result
    }

    pub fn scan_file_name_retained(&mut self) -> crate::RetainedScalarScan<ScannedFileName> {
        let result = self.scan_file_name();
        self.detach_retained_scalar(result)
    }

    fn scan_file_name_inner(&mut self) -> Result<ScannedFileName, CommandError> {
        let result = self.scan_file_name_leading();
        self.finish_scalar_call(result)
    }

    fn scan_file_name_leading(&mut self) -> Result<ScannedFileName, CommandError> {
        let mut destination = None;
        let first = loop {
            let command = match self.request_expanded_token(&mut destination) {
                Ok(DeliveryStatus::Command) => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                Ok(DeliveryStatus::End) | Ok(_) => {
                    return Err(CommandError::input_invariant());
                }
                Err(error) => return Err(error),
            };
            if !matches!(
                static_meaning(command.meaning()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                break command;
            }
        };
        let provenance = StructuredProvenance {
            primary: first.origin(),
        };
        let grouped = matches!(
            static_meaning(first.meaning()),
            Some(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        );
        // `scan_file_name` replays its first non-space token before consuming
        // the filename. TeX82 exposes this `back_input` hand-off, and it
        // keeps the group-opening case on the same ordinary delivery path.
        self.back_input(first)?;
        self.scan_file_name_characters(
            FileNameComponents::default(),
            0,
            false,
            grouped,
            provenance.primary,
        )
    }

    fn scan_file_name_characters(
        &mut self,
        mut components: FileNameComponents,
        mut character_count: usize,
        mut quoted: bool,
        grouped: bool,
        provenance: OriginId,
    ) -> Result<ScannedFileName, CommandError> {
        let mut destination = None;
        loop {
            let command = match self.request_expanded_token(&mut destination) {
                Ok(DeliveryStatus::Command) => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                Ok(DeliveryStatus::End) => break,
                Ok(_) => return Err(CommandError::input_invariant()),
                Err(error) => return Err(error),
            };
            match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                }) if grouped => {}
                Some(Meaning::CharToken { ch: '"', .. }) => quoted = !quoted,
                Some(Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                }) if grouped && !quoted => {
                    break;
                }
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }) if !grouped && !quoted => {
                    break;
                }
                Some(Meaning::CharToken { ch, .. }) => {
                    character_count += 1;
                    if character_count > FILE_NAME_POOL_CAPACITY {
                        return Err(CommandError::Fatal(crate::FatalError::overflow(
                            "pool size",
                            FILE_NAME_POOL_CAPACITY as i32,
                        )));
                    }
                    components.push_character(ch);
                }
                _ if !grouped => {
                    self.back_input(command)?;
                    break;
                }
                _ => return Err(CommandError::input_invariant()),
            }
        }
        // Web2C tex.ch [29.517] applies `search_string`/`slow_make_string`
        // independently to TeX82 §§516--520's nonempty components.
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                self.state.slow_make_string_pool_string(component);
            }
        }
        Ok(ScannedFileName {
            components,
            provenance: StructuredProvenance {
                primary: provenance,
            },
        })
    }

    /// Scans and opens one input through the borrow-scoped registered-input
    /// capability. No filesystem or host lookup escapes this boundary.
    pub fn open_registered_input(&mut self) -> Result<RegisteredInput, CommandError> {
        let mut file_name = match self.command.take_pending_input_open() {
            Some(file_name) => file_name,
            None => self.scan_file_name()?,
        };
        loop {
            let retry_file_name = file_name.clone();
            let original_name = file_name.packed();
            file_name.components.apply_default_extension(".tex");
            let has_area = !file_name.components.area.is_empty();
            let packed_name = file_name.packed();
            let attempts = crate::host::input_lookup_candidates(&packed_name, has_area);
            self.state.unsupported_host_capability();

            let mut unresolved = false;
            let mut provider_declined = false;
            for attempted_name in attempts {
                let registration = loop {
                    let mut provider_fulfillment = None;
                    let mut provider_settled = false;
                    if self.host.input(&attempted_name).is_none()
                        && !self.host.input_is_unavailable(&attempted_name)
                    {
                        let need = crate::ResourceNeed::Input {
                            name: attempted_name.clone(),
                            original_name: original_name.clone(),
                        };
                        match self
                            .host
                            .resolve_and_install_resource(self.state, &need, true)?
                        {
                            Some(crate::ResourceInstallOutcome::Fulfilled(fulfillment)) => {
                                provider_settled = true;
                                provider_fulfillment = Some(fulfillment);
                            }
                            Some(crate::ResourceInstallOutcome::Unavailable) => {
                                provider_settled = true;
                            }
                            Some(crate::ResourceInstallOutcome::Declined) => {
                                // A provider has already made its one scoped
                                // attempt. Do not probe the alias or invoke it
                                // again before the outer unwind.
                                unresolved = true;
                                provider_declined = true;
                                break None;
                            }
                            None => {}
                            Some(crate::ResourceInstallOutcome::Failed(_)) => {
                                unreachable!("failed provider outcomes are returned as errors")
                            }
                        }
                    }

                    if let Some(crate::ResourceFulfillment::Input { source, .. }) =
                        provider_fulfillment
                    {
                        break Some((source, true));
                    }

                    let Some(retained) = self.host.input(&attempted_name) else {
                        if self.host.input_is_unavailable(&attempted_name) {
                            if !provider_settled {
                                self.state
                                    .record_input_dependencies(
                                        &self.host.input_unavailable_dependencies(&attempted_name),
                                    )
                                    .map_err(|_| CommandError::input_invariant())?;
                            }
                        } else {
                            unresolved = true;
                        }
                        break None;
                    };

                    let actual = self
                        .state
                        .with_input_read_state(|input| retained.actual_use(input))
                        .map_err(|_| CommandError::input_invariant())?;
                    let Some(actual) = actual else {
                        // A generated-only retained answer lost its current
                        // output on rollback. Remove only this positive entry
                        // and resolve the current request again.
                        self.host.invalidate_input_resource(&attempted_name);
                        continue;
                    };
                    if !provider_settled {
                        let dependencies = retained.dependencies_for_actual_use(&actual);
                        self.state
                            .record_input_dependencies(&dependencies)
                            .map_err(|_| CommandError::input_invariant())?;
                    }
                    break Some((actual, provider_settled));
                };
                let Some((registration, _provider_settled)) = registration else {
                    if provider_declined {
                        break;
                    }
                    continue;
                };
                let bytes = registration.shared_bytes();
                // §537's `a_make_name_string`: tex.web records the name it
                // actually opened on the level, and later prints exactly that
                // as the transcript's `(name` -- so it is the resolved name,
                // not the name the user typed. Only the host knows what
                // resolving did: web2c's kpathsea answers a bare `child.tex`
                // found beside the job with `./child.tex`, and prints the `./`.
                // A host that reports a resolved name keeps it; one that does
                // not falls back to the name that matched.
                let resolved_name = registration
                    .name()
                    .unwrap_or(attempted_name.as_str())
                    .to_owned();
                let registration = match registration.name() {
                    Some(_) => registration,
                    None => registration.with_name(attempted_name.as_str()),
                };
                let source = self
                    .command
                    .register_source(registration)
                    .map_err(|_| CommandError::input_invariant())?;
                // e-TeX 2.6 [23.328]'s `grp_stack[in_open]:=cur_boundary;
                // if_stack[in_open]:=cond_ptr`, recorded for `\tracingnesting`'s
                // `file_warning` at this level's eventual `end_file_reading`.
                let open_depths = self.capture_source_open_depths();
                self.invalidate_delivery_freshness();
                let (_, framing_name) = self
                    .command
                    .open_registered_file_with_depths(source, open_depths)
                    .map_err(|_| CommandError::input_invariant())?;
                if let Some(name) = framing_name {
                    self.print_file_open(&name);
                }
                self.prepare_started_input()?;
                self.host.initialize_job_name(&attempted_name);
                // TeX82 §537 retains `a_make_name_string` for the opened
                // request; Web2C additionally retains its full resolved name.
                self.state.make_string_pool_string(&attempted_name);
                if resolved_name != attempted_name {
                    self.state.make_string_pool_string(&resolved_name);
                }
                if attempted_name != packed_name {
                    file_name.components.area = "TeXinputs:".to_owned();
                }
                return Ok(RegisteredInput {
                    file_name,
                    source,
                    bytes,
                });
            }
            if unresolved {
                self.command.retain_pending_input_open(retry_file_name);
                return Err(CommandError::MissingInput {
                    name: packed_name,
                    original_name,
                });
            }
            file_name = self.prompt_for_input_file_name(&file_name)?;
        }
    }

    /// TeX82 §530's `prompt_file_name("input file name", ".tex")` after
    /// the retained host has authoritatively answered that both §537 input
    /// candidates are absent.
    fn prompt_for_input_file_name(
        &mut self,
        missing: &ScannedFileName,
    ) -> Result<ScannedFileName, CommandError> {
        let context = self.command.output_open_context(self.state);
        self.state
            .printer()
            .print_nl("! I can't find file `")
            .print(&missing.packed())
            .print("'.")
            .print_rendered(&context)
            .print_nl("Please type another input file name");

        if !self.state.interaction_permits_terminal_input() {
            let help = "*** (job aborted, file error in nonstop mode)";
            let mut report = self.state.print_err("Emergency stop");
            report.help(&[help]).context(context);
            report.succumb();
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "job aborted, file error in nonstop mode",
            )));
        }

        let Some(line) = self
            .state
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: ": " })
        else {
            let help = "End of file on the terminal!";
            let mut report = self.state.print_err("Emergency stop");
            report.help(&[help]).context(context);
            report.succumb();
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(help)));
        };
        self.file_name_from_terminal_line(&line)
    }

    fn file_name_from_terminal_line(
        &mut self,
        line: &str,
    ) -> Result<ScannedFileName, CommandError> {
        let mut components = FileNameComponents::default();
        let mut quoted = false;
        let mut character_count = 0usize;
        for ch in line.chars().skip_while(|ch| *ch == ' ') {
            if ch == '"' {
                quoted = !quoted;
                continue;
            }
            if ch == ' ' && !quoted {
                break;
            }
            character_count += 1;
            if character_count > FILE_NAME_POOL_CAPACITY {
                return Err(CommandError::Fatal(crate::FatalError::overflow(
                    "pool size",
                    FILE_NAME_POOL_CAPACITY as i32,
                )));
            }
            components.push_character(ch);
        }
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                self.state.slow_make_string_pool_string(component);
            }
        }
        Ok(ScannedFileName {
            components,
            provenance: StructuredProvenance {
                primary: OriginId::UNKNOWN,
            },
        })
    }

    /// TeX82 §1215's `repeat get_token until cur_tok<>space_token`.
    ///
    /// This tests the raw spelling, not `cur_cmd`: a control sequence whose
    /// current meaning is a space remains a legal definition target.
    pub(super) fn next_non_space_raw_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let delivery = self.get_token_into(destination)?;
            if delivery == DeliveryStatus::End {
                return Ok(DeliveryStatus::End);
            }
            if delivery != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            if !matches!(
                destination
                    .as_ref()
                    .ok_or(CommandError::input_invariant())?
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }

    /// TeX82 §404's expanded nonblank/non-relax fetch, delivered directly
    /// into the structured scanner operation that will classify or hand off
    /// the command.
    pub(super) fn next_non_blank_non_relax_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let delivery = self.request_expanded_token(destination)?;
            if delivery == DeliveryStatus::End {
                return Ok(DeliveryStatus::End);
            }
            if delivery != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            if !matches!(
                static_meaning(
                    destination
                        .as_ref()
                        .ok_or(CommandError::input_invariant())?
                        .meaning()
                ),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }
}
