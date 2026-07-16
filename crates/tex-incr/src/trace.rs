//! Derived summaries for ordered persistent execution traces.

use std::collections::BTreeMap;
use std::mem::size_of;

/// One dependency observation or semantic mutation in leaf execution order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceOperation<K, V> {
    Read { key: K, value: V },
    Write { key: K, value: V },
}

/// A discardable parent summary over one or more ordered trace leaves.
///
/// Reads satisfied by an earlier write are internal and do not appear in
/// `external_reads`. Redo, input, effect, and output transitions retain child
/// order exactly, so dropping this summary and replaying its leaves is
/// semantically equivalent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceSummary<K, V, I, E, O> {
    external_reads: Vec<(K, V)>,
    redo: Vec<(K, V)>,
    input: Vec<I>,
    effects: Vec<E>,
    outputs: Vec<O>,
    leaf_count: usize,
}

impl<K, V, I, E, O> TraceSummary<K, V, I, E, O>
where
    K: Clone + Ord,
    V: Clone + Eq,
    I: Clone,
    E: Clone,
    O: Clone,
{
    /// Derives a summary for one leaf from its ordered observations and writes.
    pub fn leaf(
        operations: &[TraceOperation<K, V>],
        input: &[I],
        effects: &[E],
        outputs: &[O],
    ) -> Result<Self, TraceCompositionError> {
        let mut external_reads = Vec::new();
        let mut external_positions = BTreeMap::new();
        let mut latest_writes = BTreeMap::new();
        let mut redo = Vec::new();
        compose_operations(
            operations.iter().cloned(),
            &mut external_reads,
            &mut external_positions,
            &mut latest_writes,
            &mut redo,
        )?;
        Ok(Self {
            external_reads,
            redo,
            input: input.to_vec(),
            effects: effects.to_vec(),
            outputs: outputs.to_vec(),
            leaf_count: 1,
        })
    }

    /// Composes child summaries without retaining their hierarchy.
    pub fn parent(children: &[Self]) -> Result<Self, TraceCompositionError> {
        let mut external_reads = Vec::new();
        let mut external_positions = BTreeMap::new();
        let mut latest_writes = BTreeMap::new();
        let mut redo = Vec::new();
        let mut input = Vec::new();
        let mut effects = Vec::new();
        let mut outputs = Vec::new();
        let mut leaf_count = 0_usize;

        for child in children {
            let observations = child
                .external_reads
                .iter()
                .cloned()
                .map(|(key, value)| TraceOperation::Read { key, value })
                .chain(
                    child
                        .redo
                        .iter()
                        .cloned()
                        .map(|(key, value)| TraceOperation::Write { key, value }),
                );
            compose_operations(
                observations,
                &mut external_reads,
                &mut external_positions,
                &mut latest_writes,
                &mut redo,
            )?;
            input.extend_from_slice(&child.input);
            effects.extend_from_slice(&child.effects);
            outputs.extend_from_slice(&child.outputs);
            leaf_count = leaf_count.saturating_add(child.leaf_count);
        }

        Ok(Self {
            external_reads,
            redo,
            input,
            effects,
            outputs,
            leaf_count,
        })
    }

    #[must_use]
    pub fn external_reads(&self) -> &[(K, V)] {
        &self.external_reads
    }

    #[must_use]
    pub fn redo(&self) -> &[(K, V)] {
        &self.redo
    }

    #[must_use]
    pub fn input(&self) -> &[I] {
        &self.input
    }

    #[must_use]
    pub fn effects(&self) -> &[E] {
        &self.effects
    }

    #[must_use]
    pub fn outputs(&self) -> &[O] {
        &self.outputs
    }

    #[must_use]
    pub const fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    /// Logical bytes owned directly by the derived summary vectors.
    #[must_use]
    pub fn logical_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(
                self.external_reads
                    .capacity()
                    .saturating_mul(size_of::<(K, V)>()),
            )
            .saturating_add(self.redo.capacity().saturating_mul(size_of::<(K, V)>()))
            .saturating_add(self.input.capacity().saturating_mul(size_of::<I>()))
            .saturating_add(self.effects.capacity().saturating_mul(size_of::<E>()))
            .saturating_add(self.outputs.capacity().saturating_mul(size_of::<O>()))
    }

    /// Validates every external dependency before replaying any transition.
    pub fn validate_and_replay<R, W>(
        &self,
        mut read: R,
        mut write: W,
        input: &mut Vec<I>,
        effects: &mut Vec<E>,
        outputs: &mut Vec<O>,
    ) -> Result<(), TraceValidationError>
    where
        R: FnMut(&K) -> Option<V>,
        W: FnMut(&K, &V),
    {
        for (dependency, (key, expected)) in self.external_reads.iter().enumerate() {
            if read(key).as_ref() != Some(expected) {
                return Err(TraceValidationError { dependency });
            }
        }
        for (key, value) in &self.redo {
            write(key, value);
        }
        input.extend_from_slice(&self.input);
        effects.extend_from_slice(&self.effects);
        outputs.extend_from_slice(&self.outputs);
        Ok(())
    }
}

fn compose_operations<K, V>(
    operations: impl IntoIterator<Item = TraceOperation<K, V>>,
    external_reads: &mut Vec<(K, V)>,
    external_positions: &mut BTreeMap<K, usize>,
    latest_writes: &mut BTreeMap<K, V>,
    redo: &mut Vec<(K, V)>,
) -> Result<(), TraceCompositionError>
where
    K: Clone + Ord,
    V: Clone + Eq,
{
    for operation in operations {
        match operation {
            TraceOperation::Read { key, value } => {
                if let Some(written) = latest_writes.get(&key) {
                    if written != &value {
                        return Err(TraceCompositionError::InternalReadMismatch);
                    }
                } else if let Some(position) = external_positions.get(&key).copied() {
                    if external_reads[position].1 != value {
                        return Err(TraceCompositionError::ConflictingExternalRead);
                    }
                } else {
                    external_positions.insert(key.clone(), external_reads.len());
                    external_reads.push((key, value));
                }
            }
            TraceOperation::Write { key, value } => {
                latest_writes.insert(key.clone(), value.clone());
                redo.push((key, value));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceCompositionError {
    ConflictingExternalRead,
    InternalReadMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceValidationError {
    pub dependency: usize,
}

#[cfg(test)]
mod tests;
