use std::collections::BTreeSet;

use crate::dependency::DependencyKey;
use crate::interner::Symbol;
use crate::meaning::Meaning;

/// Receives canonical state reads performed by expansion or execution.
pub trait ReadRecorder {
    fn record_meaning(&mut self, symbol: Symbol, _meaning: Meaning) {
        self.record_dependency(DependencyKey::Meaning(symbol.raw()));
    }

    fn record_dependency(&mut self, _dependency: DependencyKey) {}
}

enum StagedRecorderRead {
    Meaning(Symbol, Meaning),
    Dependency(DependencyKey),
}

/// Detached read observations produced by one candidate operation.
///
/// The batch is opaque so only the outer transaction can choose whether to
/// publish or discard it.
#[derive(Default)]
pub struct ReadRecorderBatch(Vec<StagedRecorderRead>);

impl ReadRecorderBatch {
    #[doc(hidden)]
    pub fn stage_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        self.0.push(StagedRecorderRead::Meaning(symbol, meaning));
    }

    #[doc(hidden)]
    pub fn stage_dependency(&mut self, dependency: DependencyKey) {
        self.0.push(StagedRecorderRead::Dependency(dependency));
    }

    #[doc(hidden)]
    pub fn deliver(self, recorder: &mut dyn ReadRecorder) {
        for read in self.0 {
            match read {
                StagedRecorderRead::Meaning(symbol, meaning) => {
                    recorder.record_meaning(symbol, meaning);
                }
                StagedRecorderRead::Dependency(dependency) => {
                    recorder.record_dependency(dependency);
                }
            }
        }
    }
}

/// Deterministic concrete recorder for memoization and speculation clients.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadSetRecorder {
    dependencies: BTreeSet<DependencyKey>,
}

impl ReadSetRecorder {
    #[must_use]
    pub fn dependencies(&self) -> impl ExactSizeIterator<Item = DependencyKey> + '_ {
        self.dependencies.iter().copied()
    }
}

impl ReadRecorder for ReadSetRecorder {
    fn record_dependency(&mut self, dependency: DependencyKey) {
        self.dependencies.insert(dependency);
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadRecorderBatch, ReadSetRecorder};
    use crate::dependency::DependencyKey;

    #[test]
    fn detached_reads_publish_only_when_delivered() {
        let dependency = DependencyKey::InputStack;
        let mut batch = ReadRecorderBatch::default();
        batch.stage_dependency(dependency);

        let mut recorder = ReadSetRecorder::default();
        assert_eq!(recorder.dependencies().count(), 0);
        batch.deliver(&mut recorder);
        assert_eq!(recorder.dependencies().collect::<Vec<_>>(), [dependency]);
    }
}
