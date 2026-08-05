//! Single FIFO cache for immutable classic runtime artifacts.

use std::collections::VecDeque;
use std::sync::Arc;

use super::read::{PreparedKey, prepare_classic_database};
use super::{
    ClassicDatabase, ClassicDatabaseSource, CompileLimits, CompileResult, CompiledStyle, compile,
};
use crate::{ClassicControl, ClassicDatabaseOptions};

#[derive(Clone, Debug)]
enum Entry {
    Style {
        source: Vec<u8>,
        program: Arc<CompiledStyle>,
        charge: usize,
    },
    Read {
        key: PreparedKey,
        database: Arc<ClassicDatabase>,
        charge: usize,
    },
}

impl Entry {
    const fn charge(&self) -> usize {
        match self {
            Self::Style { charge, .. } | Self::Read { charge, .. } => *charge,
        }
    }
}

/// The sole persistent, byte-charged cache for the classic runtime.
#[derive(Clone, Debug)]
pub(crate) struct ClassicRuntimeCache {
    entries: VecDeque<Entry>,
    retained_bytes: usize,
    style_bytes: usize,
    read_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl ClassicRuntimeCache {
    pub(crate) const fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            retained_bytes: 0,
            style_bytes: 0,
            read_bytes: 0,
            max_entries,
            max_bytes,
        }
    }

    pub(crate) fn set_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.max_entries = max_entries;
        self.max_bytes = max_bytes;
        self.evict_to_limits();
    }

    pub(crate) fn compile(&mut self, source: &[u8], limits: CompileLimits) -> CompileResult {
        if let Some(program) = self.entries.iter().find_map(|entry| match entry {
            Entry::Style {
                source: cached,
                program,
                ..
            } if cached == source && program.charge().fits(limits) => Some(Arc::clone(program)),
            _ => None,
        }) {
            return CompileResult::cached(program);
        }

        let result = compile(source, limits);
        if let Some(program) = result.program() {
            let charge = program.charge().retained_bytes;
            if charge <= limits.retained_cache_bytes {
                self.insert(Entry::Style {
                    source: source.to_vec(),
                    program: Arc::clone(program),
                    charge,
                });
            }
        }
        result
    }

    pub(crate) fn prepare(
        &mut self,
        control: &ClassicControl,
        style: &CompiledStyle,
        sources: &[ClassicDatabaseSource<'_>],
        options: &ClassicDatabaseOptions,
    ) -> Arc<ClassicDatabase> {
        let key = PreparedKey::new(control, style, sources, options);
        if let Some(database) = self.entries.iter().find_map(|entry| match entry {
            Entry::Read {
                key: cached,
                database,
                ..
            } if cached == &key => Some(Arc::clone(database)),
            _ => None,
        }) {
            return database;
        }

        let database = Arc::new(prepare_classic_database(control, style, sources, options));
        let charge = key
            .retained_bytes()
            .saturating_add(database.retained_bytes());
        self.insert(Entry::Read {
            key,
            database: Arc::clone(&database),
            charge,
        });
        database
    }

    pub(crate) const fn retained_bytes(&self) -> (usize, usize) {
        (self.style_bytes, self.read_bytes)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn insert(&mut self, entry: Entry) {
        let charge = entry.charge();
        if self.max_entries == 0 || self.max_bytes == 0 || charge > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.max_entries
            || self.retained_bytes.saturating_add(charge) > self.max_bytes
        {
            self.evict_oldest();
        }
        match entry {
            Entry::Style { .. } => self.style_bytes += charge,
            Entry::Read { .. } => self.read_bytes += charge,
        }
        self.retained_bytes += charge;
        self.entries.push_back(entry);
    }

    fn evict_to_limits(&mut self) {
        while self.entries.len() > self.max_entries || self.retained_bytes > self.max_bytes {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        let Some(entry) = self.entries.pop_front() else {
            self.retained_bytes = 0;
            self.style_bytes = 0;
            self.read_bytes = 0;
            return;
        };
        let charge = entry.charge();
        match entry {
            Entry::Style { .. } => self.style_bytes = self.style_bytes.saturating_sub(charge),
            Entry::Read { .. } => self.read_bytes = self.read_bytes.saturating_sub(charge),
        }
        self.retained_bytes = self.retained_bytes.saturating_sub(charge);
    }
}
