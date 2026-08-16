//! Direct-execution ceiling experiment for a macro-heavy TeX batch slice.

mod production;

use std::sync::Arc;

use tex_arith::Scaled;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont, MetricCharTag};
use tex_out::{ContentHash, PageArtifact};

pub use production::{ProductionError, run_production};
pub use tex_command::CommandWorkCounters;

pub const CHARACTER: u8 = b'A';
pub const FONT_ID: u32 = 0;
pub const CHARACTER_WIDTH: i32 = 500;
pub const CHARACTER_HEIGHT: i32 = 300;
pub const CHARACTER_DEPTH: i32 = 100;

#[derive(Debug)]
pub enum SharedBatchError {
    Fallback(tex_exec::NativeBatchFallback),
    Execute(tex_exec::NativeBatchRunError),
}

impl std::fmt::Display for SharedBatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "shared production batch failed: {self:?}")
    }
}

impl std::error::Error for SharedBatchError {}

pub fn run_shared(workload: &Workload) -> Result<BatchResult, SharedBatchError> {
    let stores = tex_state::Universe::new_with_plain_catcodes();
    let attempt = tex_exec::run_native_batch_episode(
        &stores,
        tex_exec::NativeBatchRequest {
            source: workload.source(),
            expected_calls: workload.calls(),
            profile: tex_command::CommandProfile::TEX82,
            font_id: FONT_ID,
            font: benchmark_font(),
        },
    )
    .map_err(SharedBatchError::Execute)?;
    let result = match attempt {
        tex_exec::NativeBatchAttempt::Completed(result) => result,
        tex_exec::NativeBatchAttempt::Fallback(barrier) => {
            return Err(SharedBatchError::Fallback(barrier));
        }
    };
    Ok(BatchResult {
        counts: result.counts,
        artifact: result.artifact,
        artifact_bytes: result.artifact_bytes,
        dvi: result.dvi,
        effects: result.effects,
        terminal: result.terminal,
        log: result.log,
        calls: result.calls,
        command_work: None,
    })
}

/// One complete, deterministic macro-heavy source job.
#[derive(Clone, Debug)]
pub struct Workload {
    source: Arc<[u8]>,
    calls: usize,
    relax_padding: usize,
    nested: bool,
}

impl Workload {
    #[must_use]
    pub fn new(calls: usize, relax_padding: usize) -> Self {
        Self::build(calls, relax_padding, false)
    }

    /// Builds a structurally different workload whose outer macro calls the
    /// measured macro twice with a forwarded argument.
    #[must_use]
    pub fn nested(calls: usize, relax_padding: usize) -> Self {
        assert!(calls.is_multiple_of(2), "nested calls emit pairs");
        Self::build(calls, relax_padding, true)
    }

    fn build(calls: usize, relax_padding: usize, nested: bool) -> Self {
        assert!(calls > 0, "the batch slice needs at least one macro call");
        let mut source = Vec::with_capacity(256 + calls.saturating_mul(6) + relax_padding * 6);
        source.extend_from_slice(
            br"\count0=0\count1=0\count2=0\def\e#1{\advance\count0by#1\global\advance\count1by#1\ifnum#1<5\global\advance\count2by1\else\global\advance\count2by2\fi A\kern#1sp}",
        );
        if nested {
            source.extend_from_slice(br"\def\f#1{\e{#1}\e{#1}}");
        }
        source.extend_from_slice(br"\shipout\hbox{");
        let source_calls = if nested { calls / 2 } else { calls };
        for call in 0..source_calls {
            let digit = b'1' + (call % 8) as u8;
            source.extend_from_slice(if nested { br"\f{" } else { br"\e{" });
            source.push(digit);
            source.push(b'}');
        }
        source.push(b'}');
        for _ in 0..relax_padding {
            source.extend_from_slice(br"\relax");
        }
        source.extend_from_slice(br"\end");
        Self {
            source: source.into(),
            calls,
            relax_padding,
            nested,
        }
    }

    #[must_use]
    pub fn source(&self) -> Arc<[u8]> {
        Arc::clone(&self.source)
    }

    #[must_use]
    pub const fn calls(&self) -> usize {
        self.calls
    }

    #[must_use]
    pub const fn relax_padding(&self) -> usize {
        self.relax_padding
    }

    #[must_use]
    pub fn expected_counts(&self) -> [i32; 3] {
        let mut sum = 0_i32;
        let mut branches = 0_i32;
        for call in 0..self.calls {
            let source_call = if self.nested { call / 2 } else { call };
            let value = 1 + (source_call % 8) as i32;
            sum += value;
            branches += if value < 5 { 1 } else { 2 };
        }
        [0, sum, branches]
    }
}

/// Complete semantic output of either implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchResult {
    pub counts: [i32; 3],
    pub artifact: PageArtifact,
    pub artifact_bytes: Vec<u8>,
    pub dvi: Vec<u8>,
    pub effects: Vec<tex_state::EffectRecord>,
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    pub calls: usize,
    pub command_work: Option<CommandWorkCounters>,
}

#[must_use]
pub fn benchmark_font() -> LoadedFont {
    let mut characters = vec![None; 256];
    characters[usize::from(CHARACTER)] = Some(CharMetrics {
        width: Scaled::from_raw(CHARACTER_WIDTH),
        height: Scaled::from_raw(CHARACTER_HEIGHT),
        depth: Scaled::from_raw(CHARACTER_DEPTH),
        italic_correction: Scaled::from_raw(0),
        tag: MetricCharTag::None,
    });
    LoadedFont::new(
        "batchfont",
        "batchfont.tfm",
        ContentHash::from_bytes(b"batchfont").bytes(),
        0x64b2_0008,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

pub(crate) fn serialize_dvi(
    plan: tex_out::dvi::DviPagePlan,
) -> Result<Vec<u8>, tex_out::dvi::DviError> {
    let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
    writer.write_page_plan(&plan)?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_out::PageNode;

    #[test]
    fn shared_kernel_matches_the_complete_production_result() {
        let workload = Workload::new(256, 17);
        let production = run_production(&workload).expect("production slice executes");
        let shared = run_shared(&workload).expect("shared slice executes");

        assert_eq!(production.counts, workload.expected_counts());
        assert_eq!(shared.counts, production.counts);
        assert_eq!(shared.calls, production.calls);
        assert_eq!(shared.artifact, production.artifact);
        assert_eq!(shared.artifact_bytes, production.artifact_bytes);
        assert_eq!(shared.dvi, production.dvi);
        assert_eq!(shared.effects, production.effects);
        assert_eq!(shared.terminal, production.terminal);
        assert_eq!(shared.log, production.log);
        assert_eq!(
            production
                .command_work
                .expect("production work")
                .fuel_charges,
            73 + 67 * workload.calls() as u64 + workload.relax_padding() as u64
        );
    }

    #[test]
    fn group_rollback_and_both_conditional_arms_are_observable() {
        let workload = Workload::new(8, 0);
        let result = run_shared(&workload).expect("shared slice executes");

        assert_eq!(result.counts, [0, 36, 12]);
        let PageNode::HList(root) = &result.artifact.root else {
            panic!("shipout root must be an hbox");
        };
        assert_eq!(root.children.len(), 16);
    }

    #[test]
    fn nested_argument_forwarding_matches_production_exactly() {
        let workload = Workload::nested(512, 5);
        let production = run_production(&workload).expect("nested production slice executes");
        let shared = run_shared(&workload).expect("nested shared slice executes");

        assert_eq!(shared.counts, production.counts);
        assert_eq!(shared.calls, production.calls);
        assert_eq!(shared.artifact_bytes, production.artifact_bytes);
        assert_eq!(shared.dvi, production.dvi);
        assert_eq!(shared.effects, production.effects);
        assert_eq!(shared.terminal, production.terminal);
        assert_eq!(shared.log, production.log);
    }
}
