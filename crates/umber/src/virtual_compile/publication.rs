//! Private finalization and acceptance of one completed compile candidate.
//!
//! Every fallible output and render step finishes against the candidate
//! transaction before the accepted revision becomes visible to callers.

use super::*;
use crate::memory_output::publish_auxiliary_outputs;
#[cfg(not(test))]
use tex_out::html::incremental::PatchLimits;
use tex_out::html::incremental::{
    RenderLimits, RenderSessionId, build_render_document, plan_patch,
};
use umber_vfs::GeneratedTransaction;

pub(super) struct PreparedPublication {
    output: MemoryRunOutput,
    render_document: Option<RenderDocument>,
    render_update: Option<RenderUpdate>,
}

impl<'store> VirtualCompileSession<'store> {
    pub(super) fn prepare_publication(
        &self,
        execution: &PreparedExecution<'store>,
        generated_transaction: &mut GeneratedTransaction<'_>,
    ) -> Result<PreparedPublication, CompileError> {
        let mut accepted_world = tex_state::World::memory();
        accepted_world
            .publish_detached_effect_records(execution.completion().effects())
            .map_err(|error| CompileError::Output(format!("{error:?}")))?;
        let terminal = accepted_world
            .memory_terminal_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let log = accepted_world
            .memory_log_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let files = publish_auxiliary_outputs(&accepted_world, generated_transaction)
            .map_err(map_memory_output)?;
        let dvi = if !self.outputs.contains(OutputCapability::Dvi) || execution.pages().is_empty() {
            Vec::new()
        } else {
            execution
                .dvi_bytes()
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Dvi,
                    message: error.to_string(),
                })?
        };
        let mut output = MemoryRunOutput {
            outputs: self.outputs,
            terminal,
            log,
            dvi,
            html: None,
            html_assets: Vec::new(),
            files,
        };
        let existing = output
            .terminal
            .len()
            .saturating_add(output.log.len())
            .saturating_add(output.dvi.len())
            .saturating_add(
                output
                    .files
                    .iter()
                    .map(|file| file.bytes.len())
                    .sum::<usize>(),
            );
        let remaining = self.limits.output_bytes.saturating_sub(existing);
        let mut next_render_document = None;
        let html = if self.outputs.contains(OutputCapability::Html) {
            let output_id = match execution {
                PreparedExecution::Initial { session, .. } => session.output_id(),
                PreparedExecution::Transaction(_) => self
                    .incremental
                    .as_ref()
                    .expect("a prepared patch has an accepted incremental session")
                    .output_id(),
            };
            let font_responses = self.font_response_fingerprints();
            let assets = SessionFontResolver {
                resolved: &self.resources.resolved_fonts,
                responses: &font_responses,
            };
            let html_options = tex_out::html::HtmlOptions {
                asset_mode: self.html_asset_mode.clone(),
                revision: execution.revision().raw(),
                output_id,
                max_html_bytes: remaining,
                max_total_asset_bytes: remaining,
                max_asset_bytes: remaining,
                ..tex_out::html::HtmlOptions::default()
            };
            let pages = execution
                .pages()
                .iter()
                .map(|page| tex_out::PageArtifact::from_bytes(page.artifact().bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Html,
                    message: error.to_string(),
                })?;
            let previous_render = self
                .accepted_render_document
                .as_ref()
                .filter(|document| {
                    execution.revision().raw() == document.revision.revision.saturating_add(1)
                })
                .map(|document| &document.revision);
            let render_document = build_render_document(
                &pages,
                &assets,
                &html_options,
                RenderSessionId::from_bytes(output_id.as_bytes()),
                execution.revision().raw(),
                previous_render,
                RenderLimits {
                    max_pages: html_options.max_pages,
                    max_nodes: html_options.max_positioned_events,
                    max_resources: 65_536,
                    max_resource_bytes: remaining,
                },
            )
            .map_err(|error| CompileError::OutputCapability {
                capability: OutputCapability::Html,
                message: error.to_string(),
            })?;
            let html = tex_out::html::write_render_document(&render_document, &html_options)
                .map_err(|error| CompileError::OutputCapability {
                    capability: OutputCapability::Html,
                    message: error.to_string(),
                })?;
            next_render_document = Some(render_document);
            Some(html)
        } else {
            None
        };
        if let Some(html) = html {
            let attempted = existing.saturating_add(html.html.len()).saturating_add(
                html.assets
                    .iter()
                    .map(|asset| asset.bytes.len())
                    .sum::<usize>(),
            );
            check_limit("returned output bytes", attempted, self.limits.output_bytes)?;
            output.html = Some(html.html);
            output.html_assets = html
                .assets
                .into_iter()
                .map(|asset| crate::MemoryOutputFile {
                    path: asset.path.into(),
                    bytes: asset.bytes,
                })
                .collect();
        }
        check_limit("returned output bytes", existing, self.limits.output_bytes)?;
        let render_update = match &next_render_document {
            Some(target) if self.pending_render_update.is_some() => {
                // A missed acknowledgement requires an explicit new snapshot.
                Some(RenderUpdate::Snapshot(target.clone()))
            }
            Some(target) if self.accepted_render_document.is_some() => {
                let base = self
                    .accepted_render_document
                    .as_ref()
                    .expect("checked above");
                if target.revision.revision != base.revision.revision.saturating_add(1) {
                    Some(RenderUpdate::Snapshot(target.clone()))
                } else {
                    #[cfg(test)]
                    let patch_limits = self.render_patch_limits;
                    #[cfg(not(test))]
                    let patch_limits = PatchLimits::default();
                    let patch = plan_patch(&base.revision, &target.revision, patch_limits)
                        .map_err(|error| CompileError::OutputCapability {
                            capability: OutputCapability::Html,
                            message: error.to_string(),
                        })?;
                    Some(RenderUpdate::Patch(patch))
                }
            }
            Some(target) => Some(RenderUpdate::Snapshot(target.clone())),
            None => None,
        };
        Ok(PreparedPublication {
            output,
            render_document: next_render_document,
            render_update,
        })
    }

    pub(super) fn accept_publication(
        &mut self,
        execution: PreparedExecution<'store>,
        pending_workspace: ProjectWorkspace,
        publication: PreparedPublication,
    ) -> Result<CompileAttemptResult, CompileError> {
        // The generated set is accepted only inside the still-private pending
        // workspace. Every remaining fallible operation precedes the visible
        // installation of accepted output, workspace, and render delivery.
        let previous_generated = generated_fingerprint(&self.resources.workspace)?;
        let next_generated = generated_fingerprint(&pending_workspace)?;
        let reuse = execution.reuse();
        let accepted_engine_output = match execution {
            PreparedExecution::Initial { session, accepted } => {
                self.incremental = Some(session);
                self.format = None;
                accepted
            }
            PreparedExecution::Transaction(transaction) => Box::new(
                self.incremental
                    .as_mut()
                    .expect("a prepared patch has an accepted incremental session")
                    .accept_revision(*transaction)
                    .map_err(|error| CompileError::Incremental(error.to_string()))?,
            ),
        };
        let PreparedPublication {
            output,
            render_document,
            render_update,
        } = publication;
        self.accepted_engine_output = Some(accepted_engine_output);
        self.resources.workspace = pending_workspace;
        self.pending_patch = None;
        self.last_reuse = Some(reuse);
        self.last_stabilization_required = previous_generated != next_generated;
        self.accepted_output = Some(output.clone());
        if let Some(target) = render_document {
            self.accepted_render_document = Some(target);
            self.pending_render_update = render_update;
        }
        Ok(CompileAttemptResult::Complete(output))
    }
}
