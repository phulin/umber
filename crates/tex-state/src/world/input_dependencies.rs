//! input dependencies operations on the existing World owner.

use super::*;

impl World {
    #[must_use]
    pub fn input_records(&self) -> InputRecords<'_> {
        InputRecords { world: self }
    }

    pub(super) fn accepted_input_len(&self) -> usize {
        self.accepted_inputs
            .as_ref()
            .map_or(0, |block| block.total_len)
    }

    /// Records one authoritative semantic observation of a canonical path.
    ///
    /// Repeated observations are reduced by path. Required reads dominate
    /// probes, and a later authoritative outcome replaces an earlier one.
    pub fn record_input_dependency(
        &mut self,
        path: impl Into<PathBuf>,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        let path = path.into();
        if let Some(existing) = self.input_dependency(path.as_path()).cloned() {
            let mut updated = existing;
            updated.outcome = outcome;
            if access == InputDependencyAccess::RequiredRead {
                updated.access = access;
            }
            self.journal_input_dependency(path.as_path());
            Arc::make_mut(&mut self.input_dependencies).insert(updated.path.clone(), updated);
            return Ok(());
        }
        if self.input_dependency_len == MAX_INPUT_DEPENDENCIES {
            return Err(WorldError::new(
                "record input dependency",
                Some(path),
                format!("distinct input dependency limit {MAX_INPUT_DEPENDENCIES} exceeded"),
            ));
        }
        let path: Arc<Path> = Arc::from(path.into_boxed_path());
        self.journal_input_dependency(path.as_ref());
        Arc::make_mut(&mut self.input_dependencies).insert(
            Arc::clone(&path),
            InputDependency {
                path,
                outcome,
                access,
            },
        );
        self.input_dependency_len += 1;
        Ok(())
    }

    /// Enumerates reduced dependencies in canonical path order.
    pub fn input_dependencies(&self) -> impl Iterator<Item = InputDependency> {
        self.input_dependency_values().into_iter()
    }

    fn input_dependency(&self, path: &Path) -> Option<&InputDependency> {
        self.input_dependencies
            .get(path)
            .or_else(|| self.accepted_input_dependencies.as_ref()?.get(path))
    }

    pub(super) fn input_dependency_values(&self) -> Vec<InputDependency> {
        let mut merged = BTreeMap::new();
        if let Some(accepted) = &self.accepted_input_dependencies {
            accepted.merge_into(&mut merged);
        }
        merged.extend(
            self.input_dependencies
                .iter()
                .map(|(path, value)| (Arc::clone(path), value.clone())),
        );
        merged.into_values().collect()
    }

    fn journal_input_dependency(&mut self, path: &Path) {
        self.detached
            .reserve_input_dependency(self.input_dependency_journal.len() + 1);
        let previous = self.input_dependencies.get(path).cloned();
        let path = self
            .input_dependencies
            .get_key_value(path)
            .map_or_else(|| Arc::from(path), |(path, _)| Arc::clone(path));
        Arc::make_mut(&mut self.input_dependency_journal).push((path, previous));
    }

    pub(super) fn rollback_input_dependencies(&mut self, mark: usize) {
        if self.input_dependency_journal.len() == mark {
            return;
        }
        let journal = Arc::make_mut(&mut self.input_dependency_journal);
        for (path, previous) in journal[mark..].iter().rev() {
            match previous {
                Some(value) => {
                    Arc::make_mut(&mut self.input_dependencies)
                        .insert(Arc::clone(path), value.clone());
                }
                None => {
                    Arc::make_mut(&mut self.input_dependencies).remove(path.as_ref());
                }
            }
        }
        journal.truncate(mark);
    }

    /// Enumerates only immutable external dependencies, excluding files
    /// generated and reopened transactionally by this TeX run.
    pub fn external_input_records(&self) -> impl Iterator<Item = &InputRecord> {
        self.input_records()
            .iter()
            .filter(|record| record.is_external_dependency())
    }

    /// Verifies that every pinned included/font input still names the same
    /// host bytes before a retained checkpoint is reused.
    pub fn validate_recorded_inputs(&self) -> Result<(), WorldError> {
        for record in self.external_input_records() {
            let current = match &self.backend {
                WorldBackend::Real { .. } => std::fs::read(record.path()).map_err(|error| {
                    WorldError::from_io_error(
                        "validate retained input",
                        Some(record.path().to_owned()),
                        &error,
                    )
                })?,
                WorldBackend::Memory(memory) => memory
                    .files
                    .get(record.path())
                    .map(|bytes| bytes.to_vec())
                    .ok_or_else(|| {
                        WorldError::new(
                            "validate retained input",
                            Some(record.path().to_owned()),
                            "input is no longer available",
                        )
                    })?,
            };
            if ContentHash::from_bytes(&current) != record.hash() {
                return Err(WorldError::new(
                    "validate retained input",
                    Some(record.path().to_owned()),
                    "input content changed since the accepted checkpoint",
                ));
            }
        }
        Ok(())
    }

    /// Returns a recorded input only when `id` is live in this World timeline.
    #[must_use]
    pub fn input_record(&self, id: InputRecordId) -> Option<&InputRecord> {
        if !self.input_identities.contains(id.0) {
            return None;
        }
        self.input_records().get(id.raw() as usize)
    }

    /// Returns the content-addressed bytes for a previously-read input.
    #[must_use]
    pub fn input_content(&self, hash: ContentHash) -> Option<&[u8]> {
        self.input_contents
            .get(&hash)
            .map(AsRef::as_ref)
            .or_else(|| self.accepted_inputs.as_ref()?.content(hash))
    }

    pub(super) fn input_content_root(&self, hash: ContentHash) -> Option<SharedBytes> {
        self.input_contents
            .get(&hash)
            .cloned()
            .or_else(|| self.accepted_inputs.as_ref()?.content_root(hash))
    }
}
