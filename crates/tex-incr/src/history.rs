//! Accepted named-boundary history and pruning policy.

use std::time::Duration;

use tex_exec::EngineBoundary;

use crate::{ReuseMetrics, RevisionExecutionPath, RevisionId, SameHistoryStop, Timer};

/// Executor-owned occurrence key for one named boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundaryKey {
    pub position: usize,
    pub boundary: EngineBoundary,
    pub ordinal: u32,
}

/// Handle-free accepted observation of one named runtime boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryRecord {
    pub(crate) revision: RevisionId,
    pub(crate) key: BoundaryKey,
    pub(crate) effect_prefix: usize,
    pub(crate) artifact_prefix: usize,
    pub(crate) state_hash: u64,
}

impl BoundaryRecord {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn key(&self) -> BoundaryKey {
        self.key
    }

    #[must_use]
    pub const fn artifact_prefix(&self) -> usize {
        self.artifact_prefix
    }

    #[must_use]
    pub const fn effect_prefix(&self) -> usize {
        self.effect_prefix
    }

    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }
}

pub(crate) struct HistoryComparison<'a> {
    pub(crate) execution_path: RevisionExecutionPath,
    pub(crate) old: &'a [BoundaryRecord],
    pub(crate) new: &'a [BoundaryRecord],
    pub(crate) unchanged_content: bool,
    pub(crate) source_len: usize,
    pub(crate) delivered_commands: usize,
    pub(crate) revision_setup_latency: Duration,
    pub(crate) pages_retyped: usize,
}

pub(crate) fn compare_histories(comparison: HistoryComparison<'_>) -> ReuseMetrics {
    let HistoryComparison {
        execution_path,
        old,
        new,
        unchanged_content,
        source_len,
        delivered_commands,
        revision_setup_latency,
        pages_retyped,
    } = comparison;
    if execution_path == RevisionExecutionPath::Cold || old.is_empty() {
        return ReuseMetrics {
            execution_path,
            pages_retyped,
            reexecuted_bytes: source_len,
            reexecuted_tokens: delivered_commands,
            reexecuted_commands: delivered_commands,
            reexecuted_paragraphs: paragraph_count(new),
            revision_setup_latency,
            trace_retained_bytes: std::mem::size_of_val(new),
            ..ReuseMetrics::default()
        };
    }
    if !unchanged_content {
        return ReuseMetrics {
            execution_path,
            pages_retyped,
            reexecuted_bytes: source_len,
            reexecuted_tokens: delivered_commands,
            reexecuted_commands: delivered_commands,
            reexecuted_paragraphs: paragraph_count(new),
            same_history_stop: SameHistoryStop::HashesDiverged,
            revision_setup_latency,
            trace_retained_bytes: std::mem::size_of_val(new),
            ..ReuseMetrics::default()
        };
    }
    let started = Timer::start();
    let mut attempts = 0usize;
    let mut mismatches = 0usize;
    let mut convergence = None;
    for (old_record, new_record) in old.iter().zip(new) {
        if old_record.key.boundary != new_record.key.boundary {
            continue;
        }
        attempts = attempts.saturating_add(1);
        if old_record.key == new_record.key && old_record.state_hash == new_record.state_hash {
            convergence.get_or_insert(new_record.key);
        } else {
            mismatches = mismatches.saturating_add(1);
        }
    }
    ReuseMetrics {
        execution_path,
        convergence_boundary: convergence,
        pages_retyped,
        reexecuted_bytes: source_len,
        reexecuted_tokens: delivered_commands,
        reexecuted_commands: delivered_commands,
        reexecuted_paragraphs: paragraph_count(new),
        same_history_attempts: attempts,
        same_history_hash_mismatches: mismatches,
        trace_nodes_walked: attempts,
        trace_retained_bytes: std::mem::size_of_val(new),
        same_history_stop: if convergence.is_some() {
            SameHistoryStop::Matched
        } else if attempts == 0 {
            SameHistoryStop::NoComparableBoundary
        } else {
            SameHistoryStop::HashesDiverged
        },
        revision_setup_latency,
        trace_validation_latency: started.elapsed(),
        ..ReuseMetrics::default()
    }
}

/// Prunes observations before generation compaction is considered.
///
/// Job start and the newest observation remain named durable roots. Paragraph
/// checkpoints are selected first because they are the densest optional
/// restart family. The generation owner attached to each surviving record is
/// added by the phase-7 retained-generation boundary; this selection policy
/// deliberately knows nothing about row-level storage.
pub(crate) struct PrunedHistory {
    pub(crate) records: Vec<BoundaryRecord>,
    pub(crate) retained_indices: Vec<usize>,
}

pub(crate) fn prune_history(mut history: Vec<BoundaryRecord>, budget: usize) -> PrunedHistory {
    let mut retained_indices = (0..history.len()).collect::<Vec<_>>();
    while std::mem::size_of_val(history.as_slice()) > budget && history.len() > 2 {
        let newest = history.len() - 1;
        let victim = history
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != 0
                    && *index != newest
                    && record.key.boundary == EngineBoundary::OuterParagraphEnd
            })
            .or_else(|| {
                history
                    .iter()
                    .enumerate()
                    .find(|(index, _)| *index != 0 && *index != newest)
            })
            .map(|(index, _)| index);
        let Some(victim) = victim else {
            break;
        };
        history.remove(victim);
        retained_indices.remove(victim);
    }
    PrunedHistory {
        records: history,
        retained_indices,
    }
}

fn paragraph_count(history: &[BoundaryRecord]) -> usize {
    history
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count()
}
