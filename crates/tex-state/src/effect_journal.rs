use crate::{
    EffectDomain, EffectPlacementIntraOrder, EffectPublicationId, EffectPublicationRecordOrdinal,
    EffectRecord, EffectSemanticRecordOrdinal, EffectSequence,
};

/// In-session, validated ownership unit for one revision's effect ledger.
///
/// Publication identities and ordering domains are revision-local runtime
/// sidecars, not detached wire identity. They remain private and aligned with
/// the owned records so callers can splice live revision journals but cannot
/// construct positional columns that disagree about their length. Cold output
/// consumes materialized records and detaches their value payloads instead of
/// serializing this aggregate.
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
    pub(crate) fn from_parts(
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

    /// Selects one bounded range while preserving every aligned publication
    /// column as one ownership unit.
    #[must_use]
    pub fn slice(&self, range: std::ops::Range<usize>) -> Self {
        let start = range.start.min(self.len());
        let end = range.end.min(self.len()).max(start);
        macro_rules! select {
            ($field:ident) => {
                self.$field[start..end].to_vec()
            };
        }
        Self {
            records: select!(records),
            sequences: select!(sequences),
            publications: select!(publications),
            publication_record_ordinals: select!(publication_record_ordinals),
            domains: select!(domains),
            semantic_record_ordinals: select!(semantic_record_ordinals),
            placement_intra_orders: select!(placement_intra_orders),
        }
    }

    /// Concatenates validated journals without exposing their positional
    /// sidecars.
    #[must_use]
    pub fn concat(parts: &[Self]) -> Self {
        let capacity = parts.iter().map(Self::len).sum();
        let mut joined = Self {
            records: Vec::with_capacity(capacity),
            sequences: Vec::with_capacity(capacity),
            publications: Vec::with_capacity(capacity),
            publication_record_ordinals: Vec::with_capacity(capacity),
            domains: Vec::with_capacity(capacity),
            semantic_record_ordinals: Vec::with_capacity(capacity),
            placement_intra_orders: Vec::with_capacity(capacity),
        };
        for part in parts {
            joined.records.extend_from_slice(&part.records);
            joined.sequences.extend_from_slice(&part.sequences);
            joined.publications.extend_from_slice(&part.publications);
            joined
                .publication_record_ordinals
                .extend_from_slice(&part.publication_record_ordinals);
            joined.domains.extend_from_slice(&part.domains);
            joined
                .semantic_record_ordinals
                .extend_from_slice(&part.semantic_record_ordinals);
            joined
                .placement_intra_orders
                .extend_from_slice(&part.placement_intra_orders);
        }
        joined
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
}
