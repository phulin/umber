use crate::{
    EffectDomain, EffectPlacementIntraOrder, EffectPublicationId, EffectPublicationRecordOrdinal,
    EffectRecord, EffectSemanticRecordOrdinal, EffectSequence,
};

/// Detached, validated ownership unit for one revision's effect ledger.
///
/// Publication metadata is deliberately private and aligned with the records.
/// Callers can splice journals, but cannot construct positional sidecars that
/// disagree about their length.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectJournal {
    records: Vec<EffectRecord>,
    sequences: Vec<EffectSequence>,
    publications: Vec<Option<EffectPublicationId>>,
    publication_record_ordinals: Vec<Option<EffectPublicationRecordOrdinal>>,
    domains: Vec<EffectDomain>,
    semantic_record_ordinals: Vec<EffectSemanticRecordOrdinal>,
    placement_intra_orders: Vec<EffectPlacementIntraOrder>,
}

#[cfg(test)]
mod tests;

impl EffectJournal {
    #[doc(hidden)]
    pub fn from_parts(
        records: Vec<EffectRecord>,
        sequences: Vec<EffectSequence>,
        publications: Vec<Option<EffectPublicationId>>,
        publication_record_ordinals: Vec<Option<EffectPublicationRecordOrdinal>>,
        domains: Vec<EffectDomain>,
        semantic_record_ordinals: Vec<EffectSemanticRecordOrdinal>,
        placement_intra_orders: Vec<EffectPlacementIntraOrder>,
    ) -> Option<Self> {
        let len = records.len();
        (sequences.len() == len
            && publications.len() == len
            && publication_record_ordinals.len() == len
            && domains.len() == len
            && semantic_record_ordinals.len() == len
            && placement_intra_orders.len() == len)
            .then_some(Self {
                records,
                sequences,
                publications,
                publication_record_ordinals,
                domains,
                semantic_record_ordinals,
                placement_intra_orders,
            })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn records(&self) -> &[EffectRecord] {
        &self.records
    }

    #[must_use]
    pub(crate) fn sequences(&self) -> &[EffectSequence] {
        &self.sequences
    }

    #[must_use]
    pub(crate) fn publications(&self) -> &[Option<EffectPublicationId>] {
        &self.publications
    }

    #[must_use]
    pub(crate) fn publication_record_ordinals(&self) -> &[Option<EffectPublicationRecordOrdinal>] {
        &self.publication_record_ordinals
    }

    #[must_use]
    pub(crate) fn domains(&self) -> &[EffectDomain] {
        &self.domains
    }

    #[must_use]
    pub(crate) fn semantic_record_ordinals(&self) -> &[EffectSemanticRecordOrdinal] {
        &self.semantic_record_ordinals
    }

    #[must_use]
    pub(crate) fn placement_intra_orders(&self) -> &[EffectPlacementIntraOrder] {
        &self.placement_intra_orders
    }

    /// Returns `accepted[..prefix] + live` while preserving every aligned
    /// publication column as one ownership unit.
    #[must_use]
    pub fn splice_prefix(accepted: &Self, live: &Self, prefix: usize) -> Self {
        let prefix = prefix.min(accepted.len());
        macro_rules! splice {
            ($field:ident) => {{
                let mut joined = accepted.$field[..prefix].to_vec();
                joined.extend_from_slice(&live.$field);
                joined
            }};
        }
        Self {
            records: splice!(records),
            sequences: splice!(sequences),
            publications: splice!(publications),
            publication_record_ordinals: splice!(publication_record_ordinals),
            domains: splice!(domains),
            semantic_record_ordinals: splice!(semantic_record_ordinals),
            placement_intra_orders: splice!(placement_intra_orders),
        }
    }

    /// Canonical externally visible record order. Terminal phases are placed
    /// only after their publication transaction has assigned an ordering.
    #[must_use]
    pub fn materialized_records(&self) -> Vec<EffectRecord> {
        let (ordinary, mut terminal): (Vec<_>, Vec<_>) = self
            .records
            .iter()
            .zip(&self.domains)
            .partition(|(_, domain)| !matches!(domain, EffectDomain::TerminalPublication { .. }));
        terminal.sort_by_key(|(_, domain)| match domain {
            EffectDomain::TerminalPublication {
                phase, intra_order, ..
            } => (*phase, *intra_order),
            _ => unreachable!(),
        });
        ordinary
            .into_iter()
            .chain(terminal)
            .map(|(record, _)| record.clone())
            .collect()
    }

    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        Vec<EffectRecord>,
        Vec<EffectSequence>,
        Vec<Option<EffectPublicationId>>,
        Vec<Option<EffectPublicationRecordOrdinal>>,
        Vec<EffectDomain>,
        Vec<EffectSemanticRecordOrdinal>,
        Vec<EffectPlacementIntraOrder>,
    ) {
        (
            self.records,
            self.sequences,
            self.publications,
            self.publication_record_ordinals,
            self.domains,
            self.semantic_record_ordinals,
            self.placement_intra_orders,
        )
    }
}
