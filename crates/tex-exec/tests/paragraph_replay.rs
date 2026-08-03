use std::sync::Arc;

use tex_command::{CommandProfile, RegisteredSourceKind, SourceRegistration};
use tex_exec::{
    CanonicalMainControl, CanonicalParagraphRegion, EngineBoundary, ExecutionBudgetCounters,
    MainControlStep,
};
use tex_state::Universe;

fn register_source(control: &mut CanonicalMainControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

fn run_to_end(control: &mut CanonicalMainControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

fn editor_layout_for(bytes: &[u8]) -> (tex_state::FragmentStore, tex_state::EditorLayout) {
    let mut fragments = tex_state::FragmentStore::new();
    let (fragment, _) = fragments
        .append(Arc::from(bytes), 2)
        .expect("editor fragment installs");
    let length = u32::try_from(bytes.len()).expect("fixture fits editor layout");
    let layout = tex_state::EditorLayout::new(
        "<editor>",
        tex_state::LayoutGeneration::new(2),
        vec![tex_state::Piece::new(fragment, 0, length)],
        &fragments,
    )
    .expect("editor layout installs");
    (fragments, layout)
}

fn fork_after_first_paragraph(
    old: &[u8],
    revised: Arc<[u8]>,
) -> (CanonicalMainControl, Universe, CanonicalParagraphRegion) {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, old);
    let checkpoint = loop {
        assert!(
            !matches!(
                control.step(&mut stores).expect("cold source executes"),
                MainControlStep::End | MainControlStep::EndOfInput
            ),
            "first paragraph boundary must precede end"
        );
        if control
            .take_completed_boundaries()
            .contains(&EngineBoundary::OuterParagraphEnd)
        {
            break control
                .capture_checkpoint_with_exact_identity(
                    EngineBoundary::OuterParagraphEnd,
                    &mut stores,
                    ExecutionBudgetCounters::default(),
                )
                .expect("paragraph boundary checkpoints");
        }
    };
    let _ = control.take_finished_paragraph_regions();
    run_to_end(&mut control, &mut stores);
    let suffix = control.take_finished_paragraph_regions();
    let edit_start = old
        .iter()
        .zip(revised.iter())
        .position(|(old, new)| old != new)
        .expect("fixture has one edit");
    let region = suffix
        .last()
        .expect("stable suffix paragraph records")
        .rehome_edited_root(old, Arc::clone(&revised), edit_start..edit_start + 4)
        .expect("stable suffix rehomes");
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    let (forked, _) = checkpoint
        .fork_canonical_editor(&mut replay, &substrate, old, revised, &fragments, &layout)
        .expect("canonical editor checkpoint forks");
    (replay, forked, region)
}

#[test]
fn canonical_checkpoint_fork_keeps_rehomed_suffix_replay_key() {
    let old = br"first\par
beta\par
stable suffix\par
\end";
    let revised: Arc<[u8]> = Arc::from(
        &br"first\par
delta\par
stable suffix\par
\end"[..],
    );
    let (mut replay, mut stores, region) = fork_after_first_paragraph(old, Arc::clone(&revised));
    replay.install_paragraph_replay_regions([region]);
    run_to_end(&mut replay, &mut stores);
    assert_eq!(stores.pure_memo_stats().paragraph_hits, 1);
    assert!(
        replay
            .take_finished_paragraph_regions()
            .iter()
            .any(|region| region.finished_lines().is_some())
    );
}

#[test]
fn canonical_job_start_fork_replays_after_unrelated_prefix_assignment() {
    let old = br"stateful \count5=41 paragraph text\par
stateful \count5=42 paragraph text\par
\end";
    let prefix = br"\count99=3 ";
    let mut revised = prefix.to_vec();
    revised.extend_from_slice(old);
    let revised: Arc<[u8]> = revised.into();

    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut cold = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut cold, old);
    let checkpoint = cold
        .capture_checkpoint_with_exact_identity(
            EngineBoundary::JobStart,
            &mut stores,
            ExecutionBudgetCounters::default(),
        )
        .expect("job start checkpoints");
    run_to_end(&mut cold, &mut stores);
    let regions = cold
        .take_finished_paragraph_regions()
        .into_iter()
        .map(|region| {
            region
                .rehome_edited_root(old, Arc::clone(&revised), 0..0)
                .expect("unchanged paragraph rehomes after prefix insertion")
        })
        .collect::<Vec<_>>();
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    let (mut stores, _) = checkpoint
        .fork_canonical_editor(
            &mut replay,
            &substrate,
            old,
            Arc::clone(&revised),
            &fragments,
            &layout,
        )
        .expect("job-start editor checkpoint forks");
    replay.install_paragraph_replay_regions(regions);
    run_to_end(&mut replay, &mut stores);
    assert_eq!(stores.pure_memo_stats().paragraph_hits, 2);
    assert_eq!(stores.count(99), 3);
}

#[test]
fn canonical_job_start_fork_rejects_changed_mutation_precondition() {
    let old = br"stateful \count5=41 paragraph text\par
\end";
    let prefix = br"\count5=99 ";
    let mut revised = prefix.to_vec();
    revised.extend_from_slice(old);
    let revised: Arc<[u8]> = revised.into();

    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut cold = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut cold, old);
    let checkpoint = cold
        .capture_checkpoint_with_exact_identity(
            EngineBoundary::JobStart,
            &mut stores,
            ExecutionBudgetCounters::default(),
        )
        .expect("job start checkpoints");
    run_to_end(&mut cold, &mut stores);
    let region = cold
        .take_finished_paragraph_regions()
        .pop()
        .expect("stateful paragraph records")
        .rehome_edited_root(old, Arc::clone(&revised), 0..0)
        .expect("unchanged paragraph input rehomes");
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    let (mut stores, _) = checkpoint
        .fork_canonical_editor(
            &mut replay,
            &substrate,
            old,
            Arc::clone(&revised),
            &fragments,
            &layout,
        )
        .expect("job-start editor checkpoint forks");
    replay.install_paragraph_replay_regions([region]);
    run_to_end(&mut replay, &mut stores);
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0);
    assert_eq!(stats.paragraph.key_misses, 1);
    assert_eq!(stores.count(5), 41, "cold execution applies the paragraph");
}
