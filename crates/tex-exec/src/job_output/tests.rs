use super::*;

use tex_state::{EffectRecord, InteractionMode, JobClock, PrintSink, Universe, World};

fn channel_text<G>(universe: &Universe<G>, sink: PrintSink) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite { sink: actual, text } if *actual == sink => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn lazy_dvi_and_log_names_follow_job_name_and_texput_default() {
    for (job_name, stem) in [("paper", "paper"), ("", "texput"), ("texput", "texput")] {
        crate::test_harness::with_nonstop_universe(|universe| {
            let mut output = JobOutput::default();

            assert!(!output.log_opened());
            let expected_dvi = format!("{stem}.dvi");
            assert_eq!(
                output.dvi_name(universe, job_name),
                Ok(expected_dvi.as_str())
            );
            assert!(
                !output.log_opened(),
                "opening DVI must not open the transcript"
            );
            let expected_log = format!("{stem}.log");
            assert_eq!(
                output.open_log(universe, job_name),
                Ok(expected_log.as_str())
            );
            assert!(output.log_opened());
        });
    }
}

#[test]
fn output_and_transcript_open_retry_preserve_canonical_selector_behavior() {
    let mut world = World::memory_with_clock(JobClock::DEFAULT);
    world.deny_memory_output("paper.dvi");
    world.deny_memory_output("paper.log");
    world
        .push_memory_terminal_line("alternate-output")
        .expect("memory terminal accepts the alternate output name");
    world
        .push_memory_terminal_line("alternate-transcript.log")
        .expect("memory terminal accepts the alternate transcript name");
    crate::test_harness::with_world_universe(world, |universe| {
        universe.set_interaction_mode(InteractionMode::ErrorStop);
        let mut output = JobOutput::default();

        assert_eq!(
            output.dvi_name(universe, "paper"),
            Ok("alternate-output.dvi")
        );
        assert_eq!(
            output.open_log(universe, "paper"),
            Ok("alternate-transcript.log")
        );
        assert!(output.log_opened());
        let terminal = channel_text(universe, PrintSink::Terminal);
        assert_eq!(
            terminal
                .matches("Please type another output file name")
                .count(),
            2
        );
        let log = channel_text(universe, PrintSink::Log);
        assert!(log.contains("alternate-output\n"));
        assert!(log.contains("alternate-transcript.log\n"));
    });

    crate::test_harness::with_nonstop_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Batch);
        universe.world_mut().deny_memory_output("paper.log");
        let mut output = JobOutput::default();
        assert_eq!(
            output.open_log(universe, "paper"),
            Err(JobOutputOpenError::NonInteractive)
        );
        assert!(!output.log_opened());
        assert!(channel_text(universe, PrintSink::Terminal).contains("paper.log"));
    });
}
