//! Candidate-local admission for native resources resolved at a command site.

use umber_vfs::{ProjectWorkspace, RequestIntent, VirtualFile};

use super::{FileRequest, ResolvedFile, ResourceResponse};

pub(super) type BlockingFileProvider<'a> =
    dyn FnMut(&FileRequest, bool) -> Result<ResourceResponse, String> + 'a;

/// The provider owns acquisition policy; this owner checks and binds each
/// answer in a shadow VFS before the active command observes its bytes.
pub(super) struct SynchronousAdmission<'a> {
    pub(super) workspace: ProjectWorkspace,
    max_resolved_bytes: usize,
    provider: &'a mut BlockingFileProvider<'a>,
    pub(super) admitted: Vec<(FileRequest, ResolvedFile)>,
}

impl<'a> SynchronousAdmission<'a> {
    pub(super) fn new(
        workspace: ProjectWorkspace,
        max_resolved_bytes: usize,
        provider: &'a mut BlockingFileProvider<'a>,
    ) -> Self {
        Self {
            workspace,
            max_resolved_bytes,
            provider,
            admitted: Vec::new(),
        }
    }

    pub(super) fn resolve(
        &mut self,
        request: &FileRequest,
        probe: bool,
    ) -> Result<Option<VirtualFile>, String> {
        if let Some(file) = self.workspace.get(request.key()) {
            return Ok(Some(file.clone()));
        }
        if self.workspace.is_unavailable(request.key()) {
            return Ok(None);
        }
        let response = (self.provider)(request, probe)?;
        self.workspace.authorize_blocking_file(
            request.key().clone(),
            if probe {
                RequestIntent::Probe
            } else {
                RequestIntent::Required
            },
        );
        match response {
            ResourceResponse::File(file) if file.request == *request.key() => {
                self.workspace
                    .provision(file.clone())
                    .map_err(|error| error.to_string())?;
                if self.workspace.resolved_bytes() > self.max_resolved_bytes {
                    return Err(format!(
                        "cached resource bytes requires {}, exceeding limit {}",
                        self.workspace.resolved_bytes(),
                        self.max_resolved_bytes
                    ));
                }
                self.admitted.push((request.clone(), file));
                Ok(self.workspace.get(request.key()).cloned())
            }
            ResourceResponse::FileUnavailable(key) if key == *request.key() => {
                self.workspace
                    .provision_unavailable(key)
                    .map_err(|error| error.to_string())?;
                Ok(None)
            }
            _ => Err("synchronous provider returned a mismatched file answer".to_owned()),
        }
    }
}
