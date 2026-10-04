//! Typed resource fulfillment borrows the actual context and host owners.

use super::*;

pub(super) fn resolve_provider<G>(
    context: &mut CommandContext<'_, G>,
    capabilities: &mut CommandHostCapabilities,
    provider: Option<&mut dyn ResourceProvider<G>>,
    attempted: Option<&mut Option<ResourceNeed>>,
    need: &ResourceNeed,
    register_texinputs_alias: bool,
) -> Result<Option<ResourceInstallOutcome>, ExecError> {
    let Some(provider) = provider else {
        return Ok(None);
    };
    let resolution = provider.resolve(context, need);
    let outcome = capabilities
        .install_resource_resolution(need, resolution, register_texinputs_alias)
        .map_err(|_| {
            ExecError::ResourceFailure(Box::new(tex_command::ResourceFailureDiagnostic {
                need: need.clone(),
                failure: tex_command::ResourceFailure::message(
                    "resource provider returned a mismatched typed answer",
                ),
            }))
        })?;
    match outcome {
        ResourceInstallOutcome::Declined => {
            if let Some(attempted) = attempted {
                *attempted = Some(need.clone());
            }
            Ok(Some(ResourceInstallOutcome::Declined))
        }
        ResourceInstallOutcome::Failed(failure) => Err(ExecError::ResourceFailure(Box::new(
            tex_command::ResourceFailureDiagnostic {
                need: need.clone(),
                failure,
            },
        ))),
        outcome => Ok(Some(outcome)),
    }
}

pub(super) fn image<G>(
    request: &mut PdfImageRequest<tex_command::AttemptTokenListId>,
    stores: &mut CommandContext<'_, G>,
    capabilities: &mut CommandHostCapabilities,
    mut provider: Option<&mut dyn ResourceProvider<G>>,
    mut attempted: Option<&mut Option<ResourceNeed>>,
) -> Result<PdfImageResource, ExecError> {
    stores.unsupported_host_capability();
    // pdfTeX checks \pdfoutput before it enters `scan_image`; in DVI
    // mode this must be the diagnostic, not a host-resource suspension.
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        return Ok(PdfImageResource::Unavailable);
    }
    apply_pdf_image_compatibility_policy(stores);
    request.page_box = pdf_image_page_box(stores, request);
    request.resolution = u32::try_from(
        stores
            .int_param(IntParam::PDF_IMAGE_RESOLUTION)
            .clamp(0, 65_535),
    )
    .expect("clamped image resolution is nonnegative");
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
            capabilities.pdf_image_with_dependencies(&host_request)
        else {
            let need = ResourceNeed::PdfImage {
                request: host_request.clone(),
            };
            match resolve_provider(
                stores,
                capabilities,
                provider.take(),
                attempted.take(),
                &need,
                false,
            )? {
                Some(ResourceInstallOutcome::Fulfilled(fulfillment)) => {
                    let ResourceFulfillment::PdfImage {
                        resource: active_resource,
                        ..
                    } = fulfillment
                    else {
                        unreachable!("image provider fulfillment was type-checked by installer")
                    };
                    refresh_pdf_image_origin(capabilities, &host_request, stores)?;
                    return Ok(*active_resource);
                }
                Some(ResourceInstallOutcome::Unavailable) => {
                    // pdfTeX's image scan must consume an authoritative
                    // absence so the apply phase can report the typed
                    // unavailable-image diagnostic. Declined and an
                    // absent provider answer remain retryable misses.
                    return Ok(PdfImageResource::Unavailable);
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
            capabilities.pdf_image_selection(&host_request)
        {
            let current_output = current_pdf_image_output(
                stores,
                &dependencies,
                std::path::Path::new(&selected_path),
            )?;
            match current_output {
                Some((path, hash)) if pdf_image_identity(&resolved_resource) == Some(hash) => {
                    tex_command::dependencies_for_actual_content(&dependencies, &path, hash)
                }
                Some(_) => {
                    capabilities.invalidate_pdf_image(&host_request);
                    continue;
                }
                None if origin == tex_state::InputOrigin::SameRunGenerated => {
                    capabilities.invalidate_pdf_image(&host_request);
                    continue;
                }
                None => dependencies,
            }
        } else {
            dependencies
        };
        record_cached_input_dependencies(stores, &dependencies_for_use)?;
        return Ok(resolved_resource);
    }
}

fn refresh_pdf_image_origin<G>(
    capabilities: &mut CommandHostCapabilities,
    request: &PdfImageRequest<()>,
    stores: &CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let Some((path, _)) = capabilities.pdf_image_selection(request) else {
        return Ok(());
    };
    let origin = stores
        .same_run_output_hash(&path)
        .map_err(ExecError::from)?
        .map_or(tex_state::InputOrigin::External, |_| {
            tex_state::InputOrigin::SameRunGenerated
        });
    capabilities.set_pdf_image_origin(request, origin);
    Ok(())
}

fn current_pdf_image_output<G>(
    stores: &CommandContext<'_, G>,
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
            .same_run_output_hash(dependency.path())
            .map_err(ExecError::from)?
        {
            return Ok(Some((dependency.path().to_owned(), hash)));
        }
    }
    stores
        .same_run_output_hash(selected_path)
        .map_err(ExecError::from)
        .map(|hash| hash.map(|hash| (selected_path.to_owned(), hash)))
}

fn pdf_image_identity(resource: &PdfImageResource) -> Option<tex_state::ContentHash> {
    match resource {
        PdfImageResource::Available(source) => Some(source.identity),
        PdfImageResource::Unavailable | PdfImageResource::Invalid(_) => None,
    }
}

fn record_cached_input_dependencies<G>(
    stores: &mut CommandContext<'_, G>,
    dependencies: &[tex_state::InputDependency],
) -> Result<(), ExecError> {
    if dependencies.is_empty() {
        return Ok(());
    }
    stores
        .record_input_dependencies(dependencies)
        .map_err(ExecError::from)
}
