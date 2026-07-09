use std::path::{Path, PathBuf};

use tex_state::{InputOpenState, World};

use super::TexInputSearchPath;

#[test]
fn area_less_input_uses_ordered_system_areas_and_records_resolved_path() {
    let mut world = World::memory();
    world
        .set_memory_file("/texlive/generic/hyphen/hyphen.tex", b"patterns".to_vec())
        .expect("seed hyphen input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new(
        "/texlive/plain/base",
        [
            PathBuf::from("/texlive/plain/base"),
            PathBuf::from("/texlive/generic/hyphen"),
        ],
    );

    let content = search
        .read(&mut universe.input_open_context(), "hyphen")
        .expect("resolve hyphen through system area");

    assert_eq!(
        content.path(),
        Path::new("/texlive/generic/hyphen/hyphen.tex")
    );
    assert_eq!(universe.world().input_records().len(), 1);
    assert_eq!(universe.world().input_records()[0].path(), content.path());
    assert_eq!(universe.world().input_records()[0].hash(), content.hash());
}

#[test]
fn user_area_wins_before_configured_system_areas() {
    let mut world = World::memory();
    world
        .set_memory_file("/job/hyphen.tex", b"local".to_vec())
        .expect("seed local input");
    world
        .set_memory_file("/tree/hyphen.tex", b"system".to_vec())
        .expect("seed system input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new("/job", [PathBuf::from("/tree")]);

    let content = search
        .read(&mut universe.input_open_context(), "hyphen.tex")
        .expect("resolve local input first");

    assert_eq!(content.path(), Path::new("/job/hyphen.tex"));
    assert_eq!(content.bytes(), b"local");
}

#[test]
fn explicit_area_does_not_fall_through_to_system_areas() {
    let mut world = World::memory();
    world
        .set_memory_file("/tree/sub/hyphen.tex", b"system".to_vec())
        .expect("seed system input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new("/job", [PathBuf::from("/tree")]);

    let err = search
        .read(&mut universe.input_open_context(), "sub/hyphen")
        .expect_err("explicit area must stay relative to the user area");

    assert!(err.contains("/job/sub/hyphen.tex"));
    assert!(!err.contains("/tree/sub/hyphen.tex"));
    assert!(universe.world().input_records().is_empty());
}

#[test]
fn absolute_input_is_used_without_area_prefixes() {
    let mut world = World::memory();
    world
        .set_memory_file("/absolute/input.tex", b"absolute".to_vec())
        .expect("seed absolute input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new("/job", [PathBuf::from("/tree")]);

    let content = search
        .read(&mut universe.input_open_context(), "/absolute/input")
        .expect("resolve absolute input");

    assert_eq!(content.path(), Path::new("/absolute/input.tex"));
}
