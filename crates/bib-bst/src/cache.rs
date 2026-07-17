//! FIFO cache for immutable compiled styles.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{CompileLimits, CompileResult, CompiledStyle, compile};

/// A byte-charged FIFO cache. Cache lookup always checks the stored program's
/// complete charge against the caller's active limits before returning a hit.
#[derive(Clone, Debug, Default)]
pub struct CompilationCache {
    entries: VecDeque<Entry>,
    retained_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}
#[derive(Clone, Debug)]
struct Entry {
    source: Vec<u8>,
    program: Arc<CompiledStyle>,
}

impl CompilationCache {
    #[must_use]
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            retained_bytes: 0,
            max_entries,
            max_bytes,
        }
    }
    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.retained_bytes = 0;
    }

    /// Applies session policy before the next job. Tightening a long-lived
    /// session evicts FIFO entries immediately, so stale permissive jobs
    /// cannot retain memory above the current policy.
    pub fn set_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.max_entries = max_entries;
        self.max_bytes = max_bytes;
        self.evict_to_limits();
    }
    #[must_use]
    pub fn compile(&mut self, source: &[u8], limits: CompileLimits) -> CompileResult {
        if let Some(program) = self
            .entries
            .iter()
            .find(|entry| entry.source == source)
            .map(|entry| Arc::clone(&entry.program))
            && program.charge().fits(limits)
        {
            return CompileResult::cached(program);
        }
        let result = compile(source, limits);
        if let Some(program) = result.program() {
            self.insert(source, Arc::clone(program), limits);
        }
        result
    }
    fn insert(&mut self, source: &[u8], program: Arc<CompiledStyle>, limits: CompileLimits) {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return;
        }
        let charge = program.charge().retained_bytes;
        if charge > self.max_bytes || charge > limits.retained_cache_bytes {
            return;
        }
        self.evict_for(charge);
        self.retained_bytes += charge;
        self.entries.push_back(Entry {
            source: source.to_vec(),
            program,
        });
    }

    fn evict_to_limits(&mut self) {
        if self.max_entries == 0 || self.max_bytes == 0 {
            self.clear();
            return;
        }
        while self.entries.len() > self.max_entries || self.retained_bytes > self.max_bytes {
            self.evict_oldest();
        }
    }

    fn evict_for(&mut self, charge: usize) {
        while self.entries.len() >= self.max_entries
            || self.retained_bytes.saturating_add(charge) > self.max_bytes
        {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        let Some(entry) = self.entries.pop_front() else {
            return;
        };
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(entry.program.charge().retained_bytes);
    }
}
