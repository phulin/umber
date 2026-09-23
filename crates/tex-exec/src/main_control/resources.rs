//! Typed host-resource resolution for the existing main-control owner.

use super::*;

impl<G> MainControl<G> {
    pub(super) fn resolve_provider_resource(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
    ) -> Result<Option<ResourceInstallOutcome>, ExecError> {
        let Some(resource_provider) = resource_provider.as_deref_mut() else {
            return Ok(None);
        };
        let resolution = {
            let mut context = stores
                .command_context()
                .expect("resource provider admission");
            resource_provider.resolve(&mut context, need)
        };
        self.capabilities
            .install_resource_resolution(need, resolution, register_texinputs_alias)
            .map(Some)
            .map_err(|_| ExecError::ResourceFailure {
                need: Box::new(need.clone()),
                failure: Box::new(tex_command::ResourceFailure::message(
                    "resource provider returned a mismatched typed answer",
                )),
            })
            .and_then(|outcome| match outcome {
                Some(ResourceInstallOutcome::Declined) => {
                    self.declined_resource_attempt = Some(need.clone());
                    Ok(Some(ResourceInstallOutcome::Declined))
                }
                Some(ResourceInstallOutcome::Failed(failure)) => Err(ExecError::ResourceFailure {
                    need: Box::new(need.clone()),
                    failure: Box::new(failure),
                }),
                other => Ok(other),
            })
    }

    pub(super) fn resolve_font_resource(
        &mut self,
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<Option<FontResource>, ExecError> {
        let ColdOperation::<G>::FontDefinition { request, .. } = scanned else {
            return Ok(None);
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let path = crate::canonical_font_resource_path(&request.name);
        loop {
            if let Some(retained) = self.capabilities.font(&path) {
                let dependencies = self
                    .capabilities
                    .font_dependencies(&path)
                    .unwrap_or_default();
                let actual = stores
                    .command_context()
                    .expect("font resource admission")
                    .with_input_read_state(|input| retained.actual_use(input, &dependencies))
                    .map_err(ExecError::World)?;
                let Some(actual) = actual else {
                    self.capabilities.invalidate_font_resource(&path);
                    continue;
                };
                if let Some(metrics) = font_metrics(&actual) {
                    let dependencies = tex_command::dependencies_for_actual_content(
                        &dependencies,
                        metrics.path(),
                        metrics.hash(),
                    );
                    Self::record_cached_input_dependencies(stores, &dependencies)?;
                } else {
                    Self::record_cached_input_dependencies(stores, &dependencies)?;
                }
                return Ok(Some(actual));
            }

            let need = ResourceNeed::Font {
                request: request.clone(),
            };
            match self.resolve_provider_resource(stores, resource_provider, &need, false)? {
                Some(ResourceInstallOutcome::Fulfilled(fulfillment)) => {
                    let ResourceFulfillment::Font { resource, .. } = fulfillment else {
                        unreachable!("font provider fulfillment was type-checked by installer")
                    };
                    return Ok(Some(*resource));
                }
                Some(ResourceInstallOutcome::Unavailable) => {
                    return Ok(Some(FontResource::Unavailable));
                }
                Some(ResourceInstallOutcome::Declined) | None => {
                    return Err(ExecError::MissingFont {
                        request: request.clone(),
                    });
                }
                Some(ResourceInstallOutcome::Failed(_)) => {
                    unreachable!("failed provider outcomes are returned as errors")
                }
            }
        }
    }

    pub(super) fn resolve_input_stream_resource(
        &mut self,
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
        let ColdOperation::<G>::InputStream { request, resource } = scanned else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let resolved_source = match request {
            RootedInputStreamRequest::Open { file_name, .. } => {
                // tex.web §1275: `if cur_ext="" then cur_ext:=".tex";
                // pack_cur_name`. The packed name is what is opened, so it is
                // written back into the request rather than recomputed.
                file_name.components.apply_default_extension(".tex");
                let packed_name = file_name.packed();
                // §1275's `if a_open_in(read_file[n])` leaves the stream
                // closed when the file does not open, but Umber resolves
                // inputs through the host first: an unregistered name
                // suspends the step so the driver can acquire it, and only a
                // host that reports the file absent reaches the closed-stream
                // outcome.
                let error_request = tex_command::FileEnquiryRequest::new(
                    packed_name.clone(),
                    tex_command::FileEnquiryIntent::OpenInProbe,
                );
                loop {
                    if let Some(retained) = self.capabilities.input_probe_resource(&packed_name) {
                        let actual = stores
                            .command_context()
                            .expect("openin probe admission")
                            .with_input_read_state(|input| retained.actual_use(input))
                            .map_err(ExecError::World)?;
                        let Some(actual) = actual else {
                            self.capabilities
                                .invalidate_input_probe_resource(&packed_name);
                            continue;
                        };
                        let dependencies = retained.dependencies_for_actual_use(&actual);
                        Self::record_cached_input_dependencies(stores, &dependencies)?;
                        break Some(actual.source().clone());
                    }
                    if self.capabilities.input_probe_is_unavailable(&packed_name) {
                        let dependencies = self.capabilities.input_probe_dependencies(&packed_name);
                        Self::record_cached_input_dependencies(stores, &dependencies)?;
                        break None;
                    }

                    let need = ResourceNeed::InputProbe {
                        request: error_request.clone(),
                    };
                    match self.resolve_provider_resource(stores, resource_provider, &need, false)? {
                        Some(ResourceInstallOutcome::Fulfilled(fulfillment)) => {
                            let ResourceFulfillment::InputProbe { resource, .. } = fulfillment
                            else {
                                unreachable!(
                                    "input probe provider fulfillment was type-checked by installer"
                                )
                            };
                            break Some(resource.source().clone());
                        }
                        Some(ResourceInstallOutcome::Unavailable) => break None,
                        Some(ResourceInstallOutcome::Declined) | None => {
                            return Err(ExecError::MissingInputProbe {
                                request: error_request.clone(),
                            });
                        }
                        Some(ResourceInstallOutcome::Failed(_)) => {
                            unreachable!("failed provider outcomes are returned as errors")
                        }
                    }
                }
            }
            RootedInputStreamRequest::Close { .. } | RootedInputStreamRequest::Read { .. } => None,
        };
        *resource = resolved_source;
        Ok(())
    }

    pub(super) fn resolve_pdf_image_resource(
        &mut self,
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
        let ColdOperation::<G>::PdfXImage { request, resource } = scanned else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        // pdfTeX checks \pdfoutput before it enters `scan_image`; in DVI
        // mode this must be the diagnostic, not a host-resource suspension.
        let mut context = stores.command_context().expect("live generation");
        if context.int_param(IntParam::PDF_OUTPUT) <= 0 {
            *resource = PdfImageResource::Unavailable;
            return Ok(());
        }
        apply_pdf_image_compatibility_policy(&mut context);
        request.page_box = pdf_image_page_box(&context, request);
        request.resolution = u32::try_from(
            context
                .int_param(IntParam::PDF_IMAGE_RESOLUTION)
                .clamp(0, 65_535),
        )
        .expect("clamped image resolution is nonnegative");
        drop(context);
        let host_request = PdfImageRequest {
            name: request.name.clone(),
            width: request.width,
            height: request.height,
            depth: request.depth,
            page: request.page.clone(),
            color_space_object: request.color_space_object,
            page_box: request.page_box,
            page_box_explicit: request.page_box_explicit,
            resolution: request.resolution,
            attr: request.attr.as_ref().map(|root| {
                root.attempt_id()
                    .expect("PDF image resource resolution precedes root preparation")
            }),
        };
        loop {
            let Some((resolved_resource, dependencies)) =
                self.capabilities.pdf_image_with_dependencies(&host_request)
            else {
                let need = ResourceNeed::PdfImage {
                    request: host_request.clone(),
                };
                match self.resolve_provider_resource(stores, resource_provider, &need, false)? {
                    Some(ResourceInstallOutcome::Fulfilled(fulfillment)) => {
                        let ResourceFulfillment::PdfImage {
                            resource: active_resource,
                            ..
                        } = fulfillment
                        else {
                            unreachable!("image provider fulfillment was type-checked by installer")
                        };
                        self.refresh_pdf_image_origin(&host_request, stores)?;
                        *resource = *active_resource;
                        return Ok(());
                    }
                    Some(ResourceInstallOutcome::Unavailable) => {
                        // pdfTeX's image scan must consume an authoritative
                        // absence so the apply phase can report the typed
                        // unavailable-image diagnostic. Declined and an
                        // absent provider answer remain retryable misses.
                        *resource = PdfImageResource::Unavailable;
                        return Ok(());
                    }
                    Some(ResourceInstallOutcome::Declined) | None => {
                        return Err(ExecError::MissingPdfImage {
                            request: host_request,
                        });
                    }
                    Some(ResourceInstallOutcome::Failed(_)) => {
                        unreachable!("failed provider outcomes are returned as errors")
                    }
                }
            };

            let dependencies_for_use = if let Some((selected_path, origin)) =
                self.capabilities.pdf_image_selection(&host_request)
            {
                let current_output = Self::current_pdf_image_output(
                    stores,
                    &dependencies,
                    std::path::Path::new(&selected_path),
                )?;
                match current_output {
                    Some((path, hash))
                        if Self::pdf_image_identity(&resolved_resource) == Some(hash) =>
                    {
                        tex_command::dependencies_for_actual_content(&dependencies, &path, hash)
                    }
                    Some(_) => {
                        self.capabilities.invalidate_pdf_image(&host_request);
                        continue;
                    }
                    None if origin == tex_state::InputOrigin::SameRunGenerated => {
                        self.capabilities.invalidate_pdf_image(&host_request);
                        continue;
                    }
                    None => dependencies,
                }
            } else {
                dependencies
            };
            Self::record_cached_input_dependencies(stores, &dependencies_for_use)?;
            *resource = resolved_resource;
            return Ok(());
        }
    }

    pub(super) fn refresh_pdf_image_origin(
        &mut self,
        request: &PdfImageRequest,
        stores: &Universe<G>,
    ) -> Result<(), ExecError> {
        let Some((path, _)) = self.capabilities.pdf_image_selection(request) else {
            return Ok(());
        };
        let origin = stores
            .world()
            .same_run_output_hash(path)
            .map_err(ExecError::World)?
            .map_or(tex_state::InputOrigin::External, |_| {
                tex_state::InputOrigin::SameRunGenerated
            });
        self.capabilities.set_pdf_image_origin(request, origin);
        Ok(())
    }

    pub(super) fn current_pdf_image_output(
        stores: &Universe<G>,
        dependencies: &[tex_state::InputDependency],
        selected_path: &std::path::Path,
    ) -> Result<Option<(PathBuf, tex_state::ContentHash)>, ExecError> {
        let selected_rank = dependencies
            .iter()
            .position(|dependency| dependency.path() == selected_path)
            .unwrap_or(dependencies.len());
        for (rank, dependency) in dependencies.iter().enumerate() {
            if dependency.path() == selected_path {
                continue;
            }
            if rank >= selected_rank {
                break;
            }
            if let Some(hash) = stores
                .world()
                .same_run_output_hash(dependency.path())
                .map_err(ExecError::World)?
            {
                return Ok(Some((dependency.path().to_owned(), hash)));
            }
        }
        stores
            .world()
            .same_run_output_hash(selected_path)
            .map_err(ExecError::World)
            .map(|hash| hash.map(|hash| (selected_path.to_owned(), hash)))
    }

    pub(super) fn pdf_image_identity(
        resource: &PdfImageResource,
    ) -> Option<tex_state::ContentHash> {
        match resource {
            PdfImageResource::Available(source) => Some(source.identity),
            PdfImageResource::Unavailable | PdfImageResource::Invalid(_) => None,
        }
    }

    pub(super) fn record_cached_input_dependencies(
        stores: &mut Universe<G>,
        dependencies: &[tex_state::InputDependency],
    ) -> Result<(), ExecError> {
        if dependencies.is_empty() {
            return Ok(());
        }
        stores
            .command_context()
            .expect("live generation")
            .record_input_dependencies(dependencies)
            .map_err(ExecError::World)
    }
}
