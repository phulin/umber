//! effect publication operations on the existing World owner.

use super::*;

impl World {
    #[must_use]
    pub fn shell_escape_records(&self) -> &[ShellEscapeRecord] {
        &self.shell_escapes
    }

    #[must_use]
    pub fn effect_pos(&self) -> EffectPos {
        EffectPos(self.effect_base.raw() + self.effects.len() as u64)
    }

    #[must_use]
    pub fn effect_records(&self) -> &[EffectRecord] {
        self.effects.as_slice()
    }

    /// Closes the live aligned effect columns into one validated in-session
    /// revision journal. Positional publication sidecars remain runtime-local;
    /// cold consumers detach the materialized records instead.
    #[must_use]
    pub fn effect_journal(&self) -> crate::EffectJournal {
        crate::EffectJournal::from_parts(
            self.effects.as_ref().clone(),
            self.effect_sequences.as_ref().clone(),
            self.effect_publications.as_ref().clone(),
            self.effect_publication_record_ordinals.as_ref().clone(),
            self.effect_domains.as_ref().clone(),
            self.effect_semantic_record_ordinals.as_ref().clone(),
            self.effect_placement_intra_orders.as_ref().clone(),
        )
        .expect("World effect columns are aligned")
    }

    /// Detaches canonical effect values and their optional rendered
    /// stream-open contexts in one ordinal-aligned projection.
    ///
    /// The contexts are already-owned diagnostic text. Runtime positions and
    /// publication sidecars remain inside this World.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_effect_records(&self) -> (Vec<EffectRecord>, Vec<Option<String>>) {
        let journal = self.effect_journal();
        let indices = journal.materialized_record_indices();
        let mut records = Vec::with_capacity(indices.len());
        let mut contexts = Vec::with_capacity(indices.len());
        for index in indices {
            let record = self.effects[index].clone();
            let context = if matches!(record, EffectRecord::StreamOpen { .. }) {
                self.stream_open_contexts[index].clone()
            } else {
                None
            };
            records.push(record);
            contexts.push(context);
        }
        (records, contexts)
    }

    /// Reinstalls one validated in-session journal's aligned runtime sidecars.
    pub fn install_effect_journal(&mut self, journal: &crate::EffectJournal) {
        self.install_effect_sequences(journal.sequences());
        self.install_effect_publications(journal.publications());
        self.install_effect_publication_record_ordinals(journal.publication_record_ordinals());
        self.install_effect_domains(journal.domains());
        self.install_effect_semantic_record_ordinals(journal.semantic_record_ordinals());
        self.install_effect_placement_intra_orders(journal.placement_intra_orders());
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_sequences(&self) -> Arc<Vec<EffectSequence>> {
        Arc::clone(&self.effect_sequences)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_publications(&self) -> Arc<Vec<Option<EffectPublicationId>>> {
        Arc::clone(&self.effect_publications)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_publication_record_ordinals(
        &self,
    ) -> Arc<Vec<Option<EffectPublicationRecordOrdinal>>> {
        Arc::clone(&self.effect_publication_record_ordinals)
    }

    /// Returns winner decisions made at completed semantic-effect commits.
    #[doc(hidden)]
    #[must_use]
    pub fn effect_publication_dispositions(&self) -> Arc<Vec<EffectPublicationDisposition>> {
        Arc::clone(&self.effect_publication_dispositions)
    }

    /// Commits the live publication as the semantic winner over the retained
    /// publication. This ledger deliberately does not describe artifact
    /// selection: artifact and effect transactions can choose differently.
    #[doc(hidden)]
    pub fn commit_effect_publication_winner(
        &mut self,
        rejected: Option<EffectPublicationId>,
        winner: EffectPublicationId,
        output_attempt: EffectOutputAttemptId,
        recursive_receipt: Option<PageOutputPublicationReceiptId>,
    ) {
        self.detached
            .reserve_effect_publication_disposition(self.effect_publication_dispositions.len() + 1);
        Arc::make_mut(&mut self.effect_publication_dispositions).push(
            EffectPublicationDisposition::new(rejected, winner, output_attempt, recursive_receipt),
        );
    }

    #[doc(hidden)]
    pub fn claim_effect_publication(
        &mut self,
        range: std::ops::Range<usize>,
        publication: EffectPublicationId,
    ) {
        let start = range.start.min(self.effect_publications.len());
        let end = range.end.min(self.effect_publications.len());
        let mut next = self.publication_counter(publication);
        self.journal_publication_counter(publication);
        let publications = Arc::make_mut(&mut self.effect_publications);
        let ordinals = Arc::make_mut(&mut self.effect_publication_record_ordinals);
        for index in start..end {
            if publications[index] == Some(publication) && ordinals[index].is_some() {
                continue;
            }
            publications[index] = Some(publication);
            next = next
                .checked_add(1)
                .expect("effect publication record ordinal exhausted");
            ordinals[index] = Some(EffectPublicationRecordOrdinal::new(next));
        }
        Arc::make_mut(&mut self.next_effect_publication_record_ordinals).insert(publication, next);
    }

    /// Reserves a stable identity in the effect-publication ledger.
    #[doc(hidden)]
    pub fn reserve_effect_publication(&mut self) -> EffectPublicationId {
        if let Some(publication) = self.active_effect_publication {
            return publication;
        }
        self.next_effect_publication_identity = self
            .next_effect_publication_identity
            .checked_add(1)
            .expect("effect publication identity exhausted");
        EffectPublicationId::new(self.next_effect_publication_identity)
    }

    #[doc(hidden)]
    pub fn extend_previous_effect_publication(&mut self, range: std::ops::Range<usize>) {
        let previous = self.effect_publications[..range.start.min(self.effect_publications.len())]
            .iter()
            .rev()
            .copied()
            .flatten()
            .next();
        if let Some(previous) = previous {
            self.claim_effect_publication(range, previous);
        }
    }

    #[doc(hidden)]
    pub fn claim_effect_publication_boundary(
        &mut self,
        range: std::ops::Range<usize>,
        source: usize,
        right: EffectPublicationId,
        output_attempt: EffectOutputAttemptId,
    ) {
        let Some(sequence) = self.effect_sequences.get(source).copied() else {
            return;
        };
        let start = range.start.min(self.effect_sequences.len());
        let end = range.end.min(self.effect_sequences.len());
        let left = self.effect_publications[..start]
            .iter()
            .rev()
            .copied()
            .flatten()
            .next();
        Arc::make_mut(&mut self.effect_sequences)[start..end].fill(sequence);
        let domain = EffectDomain::PublicationBoundary {
            left,
            right: Some(right),
            output_attempt,
        };
        Arc::make_mut(&mut self.effect_domains)[start..end].fill(domain);
        // This operation defines the complete typed record set for one
        // publication gap. A checkpoint may already contain an earlier
        // execution of the same claim, but that retained counter is not part
        // of the claim's semantic identity. Restart its local namespace so
        // replay reproduces the same per-record identities.
        self.journal_semantic_counter(domain);
        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).insert(domain, 0);
        let ordinals = (start..end)
            .map(|_| self.allocate_effect_semantic_record_ordinal(domain))
            .collect::<Vec<_>>();
        Arc::make_mut(&mut self.effect_semantic_record_ordinals)[start..end]
            .copy_from_slice(&ordinals);
    }

    #[doc(hidden)]
    pub fn install_effect_publications(&mut self, publications: &[Option<EffectPublicationId>]) {
        let mut installed = publications[..publications.len().min(self.effects.len())].to_vec();
        installed.resize(self.effects.len(), None);
        self.next_effect_publication_identity = self.next_effect_publication_identity.max(
            installed
                .iter()
                .flatten()
                .map(|publication| publication.0)
                .max()
                .unwrap_or(0),
        );
        self.effect_publications = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_publication_record_ordinals(
        &mut self,
        ordinals: &[Option<EffectPublicationRecordOrdinal>],
    ) {
        let mut installed = ordinals[..ordinals.len().min(self.effects.len())].to_vec();
        installed.resize(self.effects.len(), None);
        let existing = self
            .next_effect_publication_record_ordinals
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in existing {
            self.journal_publication_counter(key);
        }
        Arc::make_mut(&mut self.next_effect_publication_record_ordinals).clear();
        for (publication, ordinal) in self
            .effect_publications
            .iter()
            .copied()
            .zip(installed.iter().copied())
        {
            if let (Some(publication), Some(EffectPublicationRecordOrdinal(ordinal))) =
                (publication, ordinal)
            {
                Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                    .entry(publication)
                    .and_modify(|next| *next = (*next).max(ordinal))
                    .or_insert(ordinal);
            }
        }
        self.effect_publication_record_ordinals = Arc::new(installed);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_domains(&self) -> Arc<Vec<EffectDomain>> {
        Arc::clone(&self.effect_domains)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_semantic_record_ordinals(&self) -> Arc<Vec<EffectSemanticRecordOrdinal>> {
        Arc::clone(&self.effect_semantic_record_ordinals)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_placement_intra_orders(&self) -> Arc<Vec<EffectPlacementIntraOrder>> {
        Arc::clone(&self.effect_placement_intra_orders)
    }

    #[doc(hidden)]
    pub fn install_effect_placement_intra_orders(&mut self, orders: &[EffectPlacementIntraOrder]) {
        let mut installed = orders[..orders.len().min(self.effects.len())].to_vec();
        while installed.len() < self.effects.len() {
            installed.push(self.allocate_effect_placement_intra_order());
        }
        self.next_effect_placement_intra_order =
            installed.iter().map(|order| order.0).max().unwrap_or(0);
        self.effect_placement_intra_orders = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_semantic_record_ordinals(
        &mut self,
        ordinals: &[EffectSemanticRecordOrdinal],
    ) {
        let mut installed = ordinals[..ordinals.len().min(self.effects.len())].to_vec();
        for index in installed.len()..self.effects.len() {
            let domain = self.effect_domains[index];
            installed.push(self.allocate_effect_semantic_record_ordinal(domain));
        }
        let existing = self
            .next_effect_semantic_record_ordinals
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in existing {
            self.journal_semantic_counter(key);
        }
        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).clear();
        for (&domain, &ordinal) in self.effect_domains.iter().zip(&installed) {
            let domain = match domain {
                EffectDomain::World(_) => EffectDomain::World(0),
                // A publication boundary is a typed claim over the records
                // between two publication identities.  Replaying that same
                // claim must reproduce its claim-local ordinals rather than
                // continue after the accepted copy installed above.  A
                // genuinely different boundary has a different `{left,
                // right}` domain, while distinct records in this claim are
                // still numbered independently by the claiming operation.
                EffectDomain::PublicationBoundary { .. } => continue,
                domain => domain,
            };
            Arc::make_mut(&mut self.next_effect_semantic_record_ordinals)
                .entry(domain)
                .and_modify(|next| *next = (*next).max(ordinal.0))
                .or_insert(ordinal.0);
        }
        self.effect_semantic_record_ordinals = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_domains(&mut self, domains: &[EffectDomain]) {
        let mut installed = domains[..domains.len().min(self.effects.len())].to_vec();
        while installed.len() < self.effects.len() {
            installed.push(self.allocate_effect_domain());
        }
        self.next_publication_sequence = self.next_publication_sequence.max(
            self.effect_sequences
                .iter()
                .zip(&installed)
                .filter_map(|(sequence, domain)| {
                    matches!(domain, EffectDomain::TerminalPublication { .. }).then_some(sequence.0)
                })
                .max()
                .unwrap_or(0),
        );
        self.effect_domains = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_sequences(&mut self, sequences: &[EffectSequence]) {
        let mut installed = sequences[..sequences.len().min(self.effects.len())].to_vec();
        for _ in installed.len()..self.effects.len() {
            installed.push(self.allocate_effect_sequence());
        }
        self.next_effect_sequence = self.next_effect_sequence.max(
            installed
                .iter()
                .map(|sequence| sequence.0)
                .max()
                .unwrap_or(0),
        );
        self.next_publication_sequence = self.next_publication_sequence.max(
            installed
                .iter()
                .map(|sequence| sequence.0)
                .max()
                .unwrap_or(0),
        );
        self.effect_sequences = Arc::new(installed);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_root_identity(&self) -> EffectRootIdentity {
        effect_root_identity_for(&self.effects)
    }

    /// Number of effects accepted before this revision's private suffix.
    #[doc(hidden)]
    #[must_use]
    pub fn page_effect_prefix_len(&self) -> usize {
        self.accepted_effects
            .as_ref()
            .map_or(0, |block| block.total_len)
    }

    /// Visits the page-visible effect interval in canonical prefix order.
    ///
    /// A short block spine is built only when the requested interval actually
    /// intersects an accepted prefix. Named checkpoint capture, clone,
    /// restore, and fork never materialize or concatenate accepted blocks.
    #[doc(hidden)]
    pub fn visit_pending_page_effects(
        &self,
        pending_live_end: usize,
        visit: impl FnMut(usize, &EffectRecord),
    ) {
        let pending = self.pending_page_effect_range(pending_live_end);
        self.visit_page_effect_range(pending, visit);
    }

    /// Visits only the physical records intersecting one absolute page-effect
    /// interval. The page cursor normally points past every earlier shipout,
    /// so staging the next page must not revisit the retained output prefix.
    pub(super) fn visit_page_effect_range(
        &self,
        range: std::ops::Range<usize>,
        mut visit: impl FnMut(usize, &EffectRecord),
    ) -> usize {
        let prefix_len = self.page_effect_prefix_len();
        let accepted_start = range.start.min(prefix_len);
        let accepted_end = range.end.min(prefix_len);
        let mut inspected = 0;

        if accepted_start < accepted_end {
            let mut intersections = Vec::new();
            let mut block = self.accepted_effects.as_deref();
            while let Some(current) = block {
                let block_end = current.total_len;
                let block_start = block_end.saturating_sub(current.len);
                if accepted_start < block_end && accepted_end > block_start {
                    let start = accepted_start.saturating_sub(block_start);
                    let end = accepted_end.min(block_end).saturating_sub(block_start);
                    intersections.push((current, start..end));
                }
                if accepted_start >= block_start {
                    break;
                }
                block = current.parent.as_deref();
            }
            for (block, local) in intersections.into_iter().rev() {
                let absolute = block.total_len.saturating_sub(block.len) + local.start;
                for (offset, record) in block.effects[local].iter().enumerate() {
                    visit(absolute + offset, record);
                    inspected += 1;
                }
            }
        }

        let live_start = range
            .start
            .saturating_sub(prefix_len)
            .min(self.effects.len());
        let live_end = range
            .end
            .saturating_sub(prefix_len)
            .min(self.effects.len())
            .max(live_start);
        for (offset, record) in self.effects[live_start..live_end].iter().enumerate() {
            visit(prefix_len + live_start + offset, record);
            inspected += 1;
        }
        inspected
    }

    /// Prefix-or-live indices not yet embedded in a committed page, bounded
    /// by the caller's pre-shipout live-effect end.
    #[doc(hidden)]
    #[must_use]
    pub fn pending_page_effect_range(&self, pending_live_end: usize) -> std::ops::Range<usize> {
        let end = self
            .page_effect_prefix_len()
            .saturating_add(pending_live_end.min(self.effects.len()));
        self.page_effect_artifact_cursor.min(end)..end
    }

    /// Closes the page-visible effect interval after an artifact commit.
    #[doc(hidden)]
    pub fn finish_page_effect_interval(&mut self) {
        self.page_effect_artifact_cursor = self
            .page_effect_prefix_len()
            .saturating_add(self.effects.len());
    }

    pub(super) fn drain_page_effect_interval_prefix(&mut self, count: usize) {
        let prefix = self.page_effect_prefix_len();
        if self.page_effect_artifact_cursor > prefix {
            self.page_effect_artifact_cursor = prefix
                + self
                    .page_effect_artifact_cursor
                    .saturating_sub(prefix)
                    .saturating_sub(count);
        }
    }

    /// Absolute position of a page-visible prefix-or-live effect.
    #[doc(hidden)]
    #[must_use]
    pub fn page_effect_position(&self, index: usize) -> Option<EffectPos> {
        let len = self
            .page_effect_prefix_len()
            .checked_add(self.effects.len())?;
        (index < len).then(|| {
            EffectPos::from_raw(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1))
        })
    }

    /// Absolute append-only identity of one currently retained effect.
    #[must_use]
    pub fn effect_position(&self, index: usize) -> Option<EffectPos> {
        (index < self.effects.len()).then(|| {
            EffectPos(self.effect_base.raw() + u64::try_from(index).unwrap_or(u64::MAX) + 1)
        })
    }

    /// Retargets the first pending stream-open after an authoritative,
    /// retry-safe failure.
    ///
    /// Earlier effects have already been drained by [`Self::commit_effects`];
    /// the failed open and its following suffix remain ordered and untouched.
    /// TeX82 §1374 changes only the failed open's filename before retrying.
    pub fn retarget_pending_stream_open(
        &mut self,
        failed: &StreamOpenFailure,
        replacement: impl Into<PathBuf>,
    ) -> Result<(), WorldError> {
        let replacement = replacement.into();
        let (path_id, path) = self.retain_stream_path(replacement.clone());
        let next_effect_position = self.effect_base.0 + 1;
        let slot = {
            let Some(EffectRecord::StreamOpen { slot, target }) = self.effects_mut().first_mut()
            else {
                return Err(WorldError::new(
                    "retarget stream open",
                    Some(failed.path.clone()),
                    "the pending effect prefix does not begin with a stream open",
                ));
            };
            if next_effect_position != failed.position.0
                || *slot != failed.slot
                || target.path.as_ref() != failed.path
            {
                return Err(WorldError::new(
                    "retarget stream open",
                    Some(failed.path.clone()),
                    "the pending stream open identity, slot, or target is stale",
                ));
            }
            target.path = Arc::clone(&path);
            target.path_id = path_id;
            *slot
        };
        if let Some(live) = self.stream_bufs_mut().write_streams[slot.index()].as_mut() {
            live.path = path;
            live.path_id = path_id;
        }
        Ok(())
    }
}
