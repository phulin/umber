//! Direct-execution ceiling experiment for a macro-heavy TeX batch slice.

mod production;

use std::sync::Arc;

use tex_arith::Scaled;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont, MetricCharTag, font_content_hash};
use tex_out::PageArtifact;

pub use production::{ProductionError, run_production};
pub use tex_command::CommandWorkCounters;

pub const CHARACTER: u8 = b'A';
pub const FONT_ID: u32 = 0;
pub const CHARACTER_WIDTH: i32 = 500;
pub const CHARACTER_HEIGHT: i32 = 300;
pub const CHARACTER_DEPTH: i32 = 100;

/// Installs the format-level category codes needed by the synthetic Plain
/// workload without introducing a live template or compatibility Universe.
pub fn prepare_plain_catcodes<G>(stores: &mut tex_state::Universe<G>) {
    let mut context = stores.command_context().expect("fresh benchmark admission");
    for (character, catcode) in [
        ('{', tex_state::token::Catcode::BeginGroup),
        ('}', tex_state::token::Catcode::EndGroup),
        ('$', tex_state::token::Catcode::MathShift),
        ('&', tex_state::token::Catcode::AlignmentTab),
        ('#', tex_state::token::Catcode::Parameter),
        ('^', tex_state::token::Catcode::Superscript),
        ('_', tex_state::token::Catcode::Subscript),
    ] {
        context
            .assign_code(
                tex_state::CodeTableKind::Catcode,
                character,
                i64::from(catcode as u8),
                tex_state::AssignmentScope::Global,
            )
            .expect("plain category-code assignment");
    }
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

/// Complete retained output of the production episode workload.
#[derive(Debug)]
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
        font_content_hash(b"batchfont"),
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
    fn production_episode_retains_complete_output() {
        let workload = Workload::new(256, 17);
        let production = run_production(&workload).expect("production slice executes");

        assert_eq!(production.counts, workload.expected_counts());
        assert_eq!(
            production
                .command_work
                .expect("production work")
                .fuel_charges,
            73 + 67 * workload.calls() as u64 + workload.relax_padding() as u64
        );
        assert_eq!(production.calls, workload.calls());
        assert!(!production.artifact_bytes.is_empty());
        assert!(!production.dvi.is_empty());
    }

    #[test]
    fn group_rollback_and_both_conditional_arms_are_observable() {
        let workload = Workload::new(8, 0);
        let result = run_production(&workload).expect("production slice executes");

        assert_eq!(result.counts, [0, 36, 12]);
        let PageNode::HList(root) = &result.artifact.root else {
            panic!("shipout root must be an hbox");
        };
        assert_eq!(root.children.len(), 16);
    }

    #[test]
    fn nested_argument_forwarding_retains_complete_output() {
        let workload = Workload::nested(512, 5);
        let production = run_production(&workload).expect("nested production slice executes");

        assert_eq!(production.counts, workload.expected_counts());
        assert_eq!(production.calls, workload.calls());
        assert!(!production.artifact_bytes.is_empty());
        assert!(!production.dvi.is_empty());
        assert_eq!(
            production
                .command_work
                .expect("packed production work")
                .fuel_charges,
            73 + 67 * workload.calls() as u64
                + workload.relax_padding() as u64
                + 16
                + 4 * (workload.calls() / 2) as u64
        );
    }
}
