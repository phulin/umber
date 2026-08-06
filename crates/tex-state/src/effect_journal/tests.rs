use super::EffectJournal;
use crate::{
    EffectDomain, EffectPlacementIntraOrder, EffectRecord, EffectSemanticRecordOrdinal,
    EffectSequence, PrintSink, TerminalPublicationId, TerminalPublicationPhase,
};

fn write(text: &str) -> EffectRecord {
    EffectRecord::StreamWrite {
        sink: PrintSink::Terminal,
        text: text.to_owned(),
    }
}

#[test]
fn rejects_misaligned_publication_columns() {
    assert!(
        EffectJournal::from_parts(
            vec![write("record")],
            Vec::new(),
            vec![None],
            vec![None],
            vec![EffectDomain::World(1)],
            vec![EffectSemanticRecordOrdinal::new(1)],
            vec![EffectPlacementIntraOrder::new(1)],
        )
        .is_none()
    );
}

#[test]
fn splice_keeps_columns_aligned_and_materializes_terminal_phases() {
    fn journal(records: Vec<EffectRecord>, domains: Vec<EffectDomain>) -> EffectJournal {
        let len = records.len();
        EffectJournal::from_parts(
            records,
            (1..=len)
                .map(|index| EffectSequence::new(index as u64))
                .collect(),
            vec![None; len],
            vec![None; len],
            domains,
            vec![EffectSemanticRecordOrdinal::new(1); len],
            (1..=len)
                .map(|index| EffectPlacementIntraOrder::new(index as u64))
                .collect(),
        )
        .expect("aligned test journal")
    }

    let accepted = journal(
        vec![write("prefix"), write("discarded")],
        vec![EffectDomain::World(1), EffectDomain::World(2)],
    );
    let live = journal(
        vec![write("notice"), write("close")],
        vec![
            EffectDomain::TerminalPublication {
                identity: TerminalPublicationId::new(1),
                phase: TerminalPublicationPhase::Notices,
                intra_order: 1,
                committed: true,
            },
            EffectDomain::TerminalPublication {
                identity: TerminalPublicationId::new(1),
                phase: TerminalPublicationPhase::CloseOpenParens,
                intra_order: 1,
                committed: true,
            },
        ],
    );
    let joined = EffectJournal::splice_prefix(&accepted, &live, 1);
    assert_eq!(joined.len(), 3);
    assert_eq!(
        joined.materialized_records(),
        vec![write("prefix"), write("close"), write("notice")]
    );
}

#[test]
fn slice_and_concat_select_prefix_patch_and_suffix_atomically() {
    fn journal(labels: &[&str], sequence_base: u64) -> EffectJournal {
        let len = labels.len();
        EffectJournal::from_parts(
            labels.iter().map(|label| write(label)).collect(),
            (sequence_base..sequence_base + len as u64)
                .map(EffectSequence::new)
                .collect(),
            vec![None; len],
            vec![None; len],
            (sequence_base..sequence_base + len as u64)
                .map(EffectDomain::World)
                .collect(),
            vec![EffectSemanticRecordOrdinal::new(1); len],
            (sequence_base..sequence_base + len as u64)
                .map(EffectPlacementIntraOrder::new)
                .collect(),
        )
        .expect("aligned test journal")
    }

    let accepted = journal(&["prefix", "replaced", "suffix"], 1);
    let patch = journal(&["patch"], 10);
    let selected =
        EffectJournal::concat(&[accepted.slice(0..1), patch, accepted.slice(2..usize::MAX)]);

    assert_eq!(
        selected.materialized_records(),
        vec![write("prefix"), write("patch"), write("suffix")]
    );
}
