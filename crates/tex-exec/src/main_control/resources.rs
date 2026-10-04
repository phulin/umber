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
            .map_err(|_| {
                ExecError::ResourceFailure(Box::new(tex_command::ResourceFailureDiagnostic {
                    need: need.clone(),
                    failure: tex_command::ResourceFailure::message(
                        "resource provider returned a mismatched typed answer",
                    ),
                }))
            })
            .and_then(|outcome| match outcome {
                Some(ResourceInstallOutcome::Declined) => {
                    self.declined_resource_attempt = Some(need.clone());
                    Ok(Some(ResourceInstallOutcome::Declined))
                }
                Some(ResourceInstallOutcome::Failed(failure)) => Err(ExecError::ResourceFailure(
                    Box::new(tex_command::ResourceFailureDiagnostic {
                        need: need.clone(),
                        failure,
                    }),
                )),
                other => Ok(other),
            })
    }

    pub(super) fn resolve_pdf_image_request(
        &mut self,
        request: &mut tex_command::PdfImageRequest<tex_command::AttemptTokenListId>,
        resource: &mut PdfImageResource,
        stores: &mut Universe<G>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
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
            attr: None,
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
                            request: Box::new(host_request),
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
        request: &PdfImageRequest<()>,
        stores: &Universe<G>,
    ) -> Result<(), ExecError> {
        let Some((path, _)) = self.capabilities.pdf_image_selection(request) else {
            return Ok(());
        };
        let origin = stores
            .world()
            .same_run_output_hash(path)
            .map_err(ExecError::from)?
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
                .map_err(ExecError::from)?
            {
                return Ok(Some((dependency.path().to_owned(), hash)));
            }
        }
        stores
            .world()
            .same_run_output_hash(selected_path)
            .map_err(ExecError::from)
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
            .map_err(ExecError::from)
    }
}
