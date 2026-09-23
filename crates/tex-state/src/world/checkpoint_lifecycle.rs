//! checkpoint lifecycle operations on the existing World owner.

use super::*;

impl World {
    #[must_use]
    pub(crate) fn snapshot(&self) -> WorldSnapshot {
        assert!(
            self.provisional_page_output_receipts.is_empty(),
            "a provisional page-output receipt crossed a World checkpoint"
        );
        WorldSnapshot {
            effect_base: self.effect_base,
            page_effect_artifact_cursor: self.page_effect_artifact_cursor,
            effect_len: self.effects.len(),
            effect_publication_disposition_len: self.effect_publication_dispositions.len(),
            next_effect_sequence: self.next_effect_sequence,
            next_publication_sequence: self.next_publication_sequence,
            next_effect_publication_identity: self.next_effect_publication_identity,
            effect_counter_journal_len: self.effect_counter_journal.len(),
            next_effect_domain: self.next_effect_domain,
            next_effect_output_attempt_identity: self.next_effect_output_attempt_identity,
            next_effect_placement_intra_order: self.next_effect_placement_intra_order,
            next_terminal_publication_identity: self.next_terminal_publication_identity,
            effect_pos: self.effect_pos(),
            stream_bufs: self.stream_bufs.mark(),
            rng: self.rng,
            pdf_rng: self.pdf_rng.clone(),
            pdf_time_micros: self.pdf_time_micros,
            pdf_timer_origin_micros: self.pdf_timer_origin_micros,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            input_len: self.inputs.len(),
            input_identities: self.input_identities.watermark(),
            input_dependency_journal_len: self.input_dependency_journal.len(),
            input_dependency_len: self.input_dependency_len,
            shell_escape_len: self.shell_escapes.len(),
            artifact_base: self.artifact_base,
            artifact_commit_len: self.artifact_pos(),
            next_artifact_publication_identity: self.next_artifact_publication_identity,
            active_artifact_publication_group: self.active_artifact_publication_group,
            active_terminal_publication: self.active_terminal_publication,
            commit_mode: self.commit_mode,
            file_framing: self.file_framing,
            error_channel: self.error_channel.clone(),
            reachable_state_identity: self.reachable_state_identity,
        }
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        if self.reachable_state_identity.is_some() {
            return true;
        }
        if self.effect_pos() != EffectPos::default()
            || !self.inputs.is_empty()
            || self.artifact_pos() != 0
            || !self.shell_escapes.is_empty()
        {
            return false;
        }
        self.reachable_state_identity = Some(WorldReachableStateIdentity::new(self));
        true
    }

    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity.map(|root| {
            crate::state_hash::semantic_scalar_root(0x776f_726c_645f_6669, |hasher| {
                hasher.u64(root.root());
                hasher.u32(self.file_framing.open_parens());
                hasher.u64(self.error_channel.reachable_state_identity());
            })
        })
    }

    pub(super) fn replace_identity_scalar(&mut self, key: u64, old: u64, new: u64) {
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.scalars.replace(key, Some(old), Some(new));
        }
    }

    pub(super) fn record_input_identity(&mut self) {
        let record = self.inputs.last().expect("input record was just published");
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.inputs.push(stable_hash(record));
        }
    }

    pub(crate) fn assert_snapshot_retained(&self, snapshot: &WorldSnapshot) {
        assert!(
            self.snapshot_effects_are_retained(snapshot)
                && (self.artifact_base..=self.artifact_pos())
                    .contains(&snapshot.artifact_commit_len),
            "World snapshot output position has already been committed and dropped"
        );
    }

    #[must_use]
    pub(crate) fn snapshot_is_retained(&self, snapshot: &WorldSnapshot) -> bool {
        self.snapshot_effects_are_retained(snapshot)
            && (self.artifact_base..=self.artifact_pos()).contains(&snapshot.artifact_commit_len)
    }

    /// Whether a strongly owned checkpoint root can seed a new retained
    /// generation after the source timeline has published its prefix.
    #[must_use]
    pub(crate) fn snapshot_is_forkable(&self, snapshot: &WorldSnapshot) -> bool {
        snapshot.effect_pos == EffectPos(snapshot.effect_base.raw() + snapshot.effect_len as u64)
    }

    fn snapshot_effects_are_retained(&self, snapshot: &WorldSnapshot) -> bool {
        snapshot.effect_pos >= self.effect_base
            && snapshot.effect_pos
                == EffectPos(snapshot.effect_base.raw() + snapshot.effect_len as u64)
            && snapshot.effect_len <= self.effects.len()
    }

    pub(crate) fn rollback(&mut self, snapshot: &WorldSnapshot) {
        self.assert_snapshot_retained(snapshot);
        self.input_identities
            .rollback(snapshot.input_identities)
            .expect("World input identity mark must name a retained ancestor");
        self.effect_base = snapshot.effect_base;
        self.page_effect_artifact_cursor = snapshot.page_effect_artifact_cursor;
        if self.effects.len() != snapshot.effect_len {
            Arc::make_mut(&mut self.effects).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_sequences).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_publications).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_publication_record_ordinals)
                .truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_domains).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_semantic_record_ordinals).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_placement_intra_orders).truncate(snapshot.effect_len);
        }
        if self.effect_publication_dispositions.len() != snapshot.effect_publication_disposition_len
        {
            Arc::make_mut(&mut self.effect_publication_dispositions)
                .truncate(snapshot.effect_publication_disposition_len);
        }
        self.next_effect_sequence = snapshot.next_effect_sequence;
        self.next_publication_sequence = snapshot.next_publication_sequence;
        self.next_effect_publication_identity = snapshot.next_effect_publication_identity;
        self.rollback_effect_counters(snapshot.effect_counter_journal_len);
        self.next_effect_domain = snapshot.next_effect_domain;
        self.next_effect_output_attempt_identity = snapshot.next_effect_output_attempt_identity;
        self.next_effect_placement_intra_order = snapshot.next_effect_placement_intra_order;
        self.next_terminal_publication_identity = snapshot.next_terminal_publication_identity;
        self.next_artifact_publication_identity = snapshot.next_artifact_publication_identity;
        if !self.provisional_page_output_receipts.is_empty() {
            Arc::make_mut(&mut self.provisional_page_output_receipts).clear();
        }
        self.active_artifact_publication_group = snapshot.active_artifact_publication_group;
        self.active_terminal_publication = snapshot.active_terminal_publication;
        Arc::make_mut(&mut self.stream_open_contexts).truncate(snapshot.effect_len);
        self.restore_stream_bufs(snapshot.stream_bufs);
        self.rng = snapshot.rng;
        self.pdf_rng = snapshot.pdf_rng.clone();
        self.pdf_time_micros = snapshot.pdf_time_micros;
        self.pdf_timer_origin_micros = snapshot.pdf_timer_origin_micros;
        self.job_clock = snapshot.job_clock;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        if self.inputs.len() != snapshot.input_len {
            Arc::make_mut(&mut self.inputs).truncate(snapshot.input_len);
        }
        self.rollback_input_dependencies(snapshot.input_dependency_journal_len);
        self.input_dependency_len = snapshot.input_dependency_len;
        self.shell_escapes.truncate(snapshot.shell_escape_len);
        if snapshot.commit_mode == WorldCommitMode::Retained {
            let retained = snapshot
                .artifact_commit_len
                .checked_sub(self.artifact_base)
                .expect("World artifact snapshot precedes retained base");
            if self.committed_artifacts.len() != retained {
                Arc::make_mut(&mut self.committed_artifacts).truncate(retained);
                Arc::make_mut(&mut self.artifact_publications).truncate(retained);
            }
        }
        self.commit_mode = snapshot.commit_mode;
        self.file_framing = snapshot.file_framing;
        self.error_channel = snapshot.error_channel.clone();
        self.reachable_state_identity = snapshot.reachable_state_identity;
    }

    /// Rewinds the direct World owner to a rooted mark and detaches the exact
    /// accepted suffix needed for either rejection redo or promotion discard.
    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        snapshot: &WorldSnapshot,
    ) -> AcceptedWorldTail {
        self.assert_snapshot_retained(snapshot);
        assert!(self.active_effect_publication.is_none());
        assert!(self.active_effect_output_attempt.is_none());
        assert!(self.active_effect_domain.is_none());
        let head = self.snapshot();
        let mut detached = std::mem::take(&mut self.detached);
        assert!(
            detached.is_empty(),
            "World already owns a detached accepted suffix"
        );
        detached
            .effects
            .extend(Arc::make_mut(&mut self.effects).drain(snapshot.effect_len..));
        detached
            .effect_sequences
            .extend(Arc::make_mut(&mut self.effect_sequences).drain(snapshot.effect_len..));
        detached
            .effect_publications
            .extend(Arc::make_mut(&mut self.effect_publications).drain(snapshot.effect_len..));
        detached.effect_publication_record_ordinals.extend(
            Arc::make_mut(&mut self.effect_publication_record_ordinals)
                .drain(snapshot.effect_len..),
        );
        detached
            .effect_domains
            .extend(Arc::make_mut(&mut self.effect_domains).drain(snapshot.effect_len..));
        detached.effect_semantic_record_ordinals.extend(
            Arc::make_mut(&mut self.effect_semantic_record_ordinals).drain(snapshot.effect_len..),
        );
        detached.effect_placement_intra_orders.extend(
            Arc::make_mut(&mut self.effect_placement_intra_orders).drain(snapshot.effect_len..),
        );
        detached.effect_publication_dispositions.extend(
            Arc::make_mut(&mut self.effect_publication_dispositions)
                .drain(snapshot.effect_publication_disposition_len..),
        );
        detached
            .stream_open_contexts
            .extend(Arc::make_mut(&mut self.stream_open_contexts).drain(snapshot.effect_len..));

        for undo in Arc::make_mut(&mut self.effect_counter_journal)
            .drain(snapshot.effect_counter_journal_len..)
        {
            let after = match undo {
                EffectCounterUndo::Publication { key, .. } => self
                    .next_effect_publication_record_ordinals
                    .get(&key)
                    .copied(),
                EffectCounterUndo::Semantic { key, .. } => {
                    self.next_effect_semantic_record_ordinals.get(&key).copied()
                }
            };
            detached
                .effect_counters
                .push(AcceptedEffectCounterWrite { undo, after });
        }
        for write in detached.effect_counters.iter().rev() {
            match write.undo {
                EffectCounterUndo::Publication { key, previous } => match previous {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .remove(&key);
                    }
                },
                EffectCounterUndo::Semantic { key, previous } => match previous {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).remove(&key);
                    }
                },
            }
        }

        detached
            .inputs
            .extend(Arc::make_mut(&mut self.inputs).drain(snapshot.input_len..));
        let input_identities = self
            .input_identities
            .begin_checkpoint_candidate(snapshot.input_identities)
            .expect("validated World input identity mark remains rewindable");
        for (path, previous) in Arc::make_mut(&mut self.input_dependency_journal)
            .drain(snapshot.input_dependency_journal_len..)
        {
            let after = self.input_dependencies.get(path.as_ref()).cloned();
            detached
                .input_dependencies
                .push(AcceptedInputDependencyWrite {
                    path,
                    previous,
                    after,
                });
        }
        for write in detached.input_dependencies.iter().rev() {
            match &write.previous {
                Some(value) => {
                    Arc::make_mut(&mut self.input_dependencies)
                        .insert(Arc::clone(&write.path), value.clone());
                }
                None => {
                    Arc::make_mut(&mut self.input_dependencies).remove(write.path.as_ref());
                }
            }
        }
        self.input_dependency_len = snapshot.input_dependency_len;

        detached
            .shell_escapes
            .extend(self.shell_escapes.drain(snapshot.shell_escape_len..));
        let artifact_mark = snapshot
            .artifact_commit_len
            .checked_sub(self.artifact_base)
            .expect("World artifact mark follows the live base");
        detached
            .committed_artifacts
            .extend(Arc::make_mut(&mut self.committed_artifacts).drain(artifact_mark..));
        detached
            .artifact_publications
            .extend(Arc::make_mut(&mut self.artifact_publications).drain(artifact_mark..));

        self.page_effect_artifact_cursor = snapshot.page_effect_artifact_cursor;
        self.next_effect_sequence = snapshot.next_effect_sequence;
        self.next_publication_sequence = snapshot.next_publication_sequence;
        self.next_effect_publication_identity = snapshot.next_effect_publication_identity;
        self.next_effect_domain = snapshot.next_effect_domain;
        self.next_effect_output_attempt_identity = snapshot.next_effect_output_attempt_identity;
        self.next_effect_placement_intra_order = snapshot.next_effect_placement_intra_order;
        self.next_terminal_publication_identity = snapshot.next_terminal_publication_identity;
        self.next_artifact_publication_identity = snapshot.next_artifact_publication_identity;
        self.active_artifact_publication_group = snapshot.active_artifact_publication_group;
        self.active_terminal_publication = snapshot.active_terminal_publication;
        self.restore_stream_bufs(snapshot.stream_bufs);
        self.rng = snapshot.rng;
        self.pdf_rng = snapshot.pdf_rng.clone();
        self.pdf_time_micros = snapshot.pdf_time_micros;
        self.pdf_timer_origin_micros = snapshot.pdf_timer_origin_micros;
        self.job_clock = snapshot.job_clock;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        self.commit_mode = snapshot.commit_mode;
        self.file_framing = snapshot.file_framing;
        self.error_channel = snapshot.error_channel.clone();
        self.reachable_state_identity = snapshot.reachable_state_identity;
        self.detached = detached;

        AcceptedWorldTail {
            head,
            input_identities,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        root: &WorldSnapshot,
        tail: AcceptedWorldTail,
    ) {
        self.rollback(root);
        self.input_identities
            .reject_checkpoint_candidate(tail.input_identities);
        let mut detached = std::mem::take(&mut self.detached);
        Arc::make_mut(&mut self.effects).append(&mut detached.effects);
        Arc::make_mut(&mut self.effect_sequences).append(&mut detached.effect_sequences);
        Arc::make_mut(&mut self.effect_publications).append(&mut detached.effect_publications);
        Arc::make_mut(&mut self.effect_publication_record_ordinals)
            .append(&mut detached.effect_publication_record_ordinals);
        Arc::make_mut(&mut self.effect_domains).append(&mut detached.effect_domains);
        Arc::make_mut(&mut self.effect_semantic_record_ordinals)
            .append(&mut detached.effect_semantic_record_ordinals);
        Arc::make_mut(&mut self.effect_placement_intra_orders)
            .append(&mut detached.effect_placement_intra_orders);
        Arc::make_mut(&mut self.effect_publication_dispositions)
            .append(&mut detached.effect_publication_dispositions);
        Arc::make_mut(&mut self.stream_open_contexts).append(&mut detached.stream_open_contexts);
        for write in &detached.effect_counters {
            match write.undo {
                EffectCounterUndo::Publication { key, .. } => match write.after {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .remove(&key);
                    }
                },
                EffectCounterUndo::Semantic { key, .. } => match write.after {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).remove(&key);
                    }
                },
            }
        }
        Arc::make_mut(&mut self.effect_counter_journal)
            .extend(detached.effect_counters.drain(..).map(|write| write.undo));
        Arc::make_mut(&mut self.inputs).append(&mut detached.inputs);
        for write in &detached.input_dependencies {
            match &write.after {
                Some(value) => {
                    Arc::make_mut(&mut self.input_dependencies)
                        .insert(Arc::clone(&write.path), value.clone());
                }
                None => {
                    Arc::make_mut(&mut self.input_dependencies).remove(write.path.as_ref());
                }
            }
        }
        Arc::make_mut(&mut self.input_dependency_journal).extend(
            detached
                .input_dependencies
                .drain(..)
                .map(|write| (write.path, write.previous)),
        );
        self.shell_escapes.append(&mut detached.shell_escapes);
        Arc::make_mut(&mut self.committed_artifacts).append(&mut detached.committed_artifacts);
        Arc::make_mut(&mut self.artifact_publications).append(&mut detached.artifact_publications);
        self.detached = detached;
        self.rollback(&tail.head);
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, tail: AcceptedWorldTail) {
        self.input_identities
            .accept_checkpoint_candidate(tail.input_identities);
        self.detached.clear();
    }

    /// Installs a retained checkpoint into a new generation. Accepted effects
    /// become an immutable page-visible prefix, while the destination starts
    /// a fresh publishable suffix at the same absolute semantic position.
    #[cfg(any(test, feature = "profiling"))]
    fn install_checkpoint_fork(&mut self, source: &Self, snapshot: &WorldSnapshot) {
        assert!(source.snapshot_is_forkable(snapshot));
        self.input_identities = source
            .input_identities
            .fork_at(snapshot.input_identities)
            .expect("World input identity mark must name a retained ancestor");

        let accepted_len = self.page_effect_prefix_len();
        assert_eq!(
            accepted_len as u64,
            snapshot.effect_base.raw(),
            "accepted effect blocks align with the source live suffix"
        );
        self.accepted_effects =
            AcceptedEffectBlock::extend(source.accepted_effects.clone(), source, snapshot);
        assert_eq!(
            self.page_effect_prefix_len() as u64,
            snapshot.effect_pos.raw()
        );
        self.page_effect_artifact_cursor = snapshot.page_effect_artifact_cursor;

        self.effect_base = snapshot.effect_pos;
        self.effects = Arc::new(Vec::new());
        self.effect_sequences = Arc::new(Vec::new());
        self.effect_publications = Arc::new(Vec::new());
        self.effect_publication_record_ordinals = Arc::new(Vec::new());
        self.effect_domains = Arc::new(Vec::new());
        self.effect_semantic_record_ordinals = Arc::new(Vec::new());
        self.effect_placement_intra_orders = Arc::new(Vec::new());
        self.active_effect_publication = None;
        self.active_effect_output_attempt = None;
        self.active_effect_domain = None;
        self.provisional_page_output_receipts = Arc::new(BTreeMap::new());
        self.next_terminal_publication_identity = self
            .next_terminal_publication_identity
            .max(snapshot.next_terminal_publication_identity);
        self.next_artifact_publication_identity = self
            .next_artifact_publication_identity
            .max(snapshot.next_artifact_publication_identity);
        self.active_artifact_publication_group = None;
        self.active_terminal_publication = None;
        self.stream_open_contexts = Arc::new(Vec::new());
        self.next_effect_sequence = snapshot.next_effect_sequence;
        self.next_effect_publication_record_ordinals = Arc::new(BTreeMap::new());
        self.next_effect_semantic_record_ordinals = Arc::new(BTreeMap::new());
        self.effect_counter_journal = Arc::new(Vec::new());
        self.next_publication_sequence = self
            .next_publication_sequence
            .max(snapshot.next_publication_sequence);
        self.next_effect_publication_identity = self
            .next_effect_publication_identity
            .max(snapshot.next_effect_publication_identity);
        self.next_effect_domain = snapshot.next_effect_domain;
        self.next_effect_placement_intra_order = snapshot.next_effect_placement_intra_order;
        self.accepted_inputs =
            AcceptedInputBlock::extend(source.accepted_inputs.clone(), source, snapshot.input_len);
        self.inputs = Arc::new(Vec::new());
        self.restore_stream_bufs(snapshot.stream_bufs);
        self.rng = snapshot.rng;
        self.pdf_rng = snapshot.pdf_rng.clone();
        self.pdf_time_micros = snapshot.pdf_time_micros;
        self.pdf_timer_origin_micros = snapshot.pdf_timer_origin_micros;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        self.input_contents = Arc::new(BTreeMap::new());
        self.accepted_input_dependencies = Some(Arc::new(AcceptedInputDependencyBlock {
            parent: source.accepted_input_dependencies.clone(),
            values: Arc::clone(&source.input_dependencies),
            journal: Arc::clone(&source.input_dependency_journal),
            journal_len: snapshot.input_dependency_journal_len,
        }));
        self.input_dependencies = Arc::new(BTreeMap::new());
        self.input_dependency_journal = Arc::new(Vec::new());
        self.input_dependency_len = snapshot.input_dependency_len;
        self.shell_escapes.truncate(snapshot.shell_escape_len);
        if snapshot.commit_mode == WorldCommitMode::Retained {
            self.artifact_base = snapshot.artifact_commit_len;
            self.committed_artifacts = Arc::new(Vec::new());
            self.artifact_publications = Arc::new(Vec::new());
        }
        self.commit_mode = snapshot.commit_mode;
        self.file_framing = snapshot.file_framing;
        self.error_channel = snapshot.error_channel.clone();
        self.reachable_state_identity = snapshot.reachable_state_identity;
    }

    /// Builds one isolated revision suffix from a retained mark without
    /// copying the accepted effect ledger.
    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn fork_checkpoint(&self, snapshot: &WorldSnapshot) -> Self {
        assert!(self.snapshot_is_forkable(snapshot));
        let mut fork = self.clone();
        fork.install_checkpoint_fork(self, snapshot);
        fork
    }
}
