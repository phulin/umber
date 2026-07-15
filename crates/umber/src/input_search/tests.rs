use std::path::{Path, PathBuf};

use tex_state::{InputOpenState, World};

use super::{TexFontSearchPath, TexInputSearchPath};

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
fn input_with_non_tex_extension_falls_back_to_appended_tex_extension() {
    let mut world = World::memory();
    world
        .set_memory_file("/tree/lipsum.ltd.tex", b"dummy text".to_vec())
        .expect("seed extension fallback input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new("/job", [PathBuf::from("/tree")]);

    let content = search
        .read(&mut universe.input_open_context(), "lipsum.ltd")
        .expect("resolve appended tex extension");

    assert_eq!(content.path(), Path::new("/tree/lipsum.ltd.tex"));
}

#[test]
fn input_with_non_tex_extension_prefers_the_exact_name() {
    let mut world = World::memory();
    world
        .set_memory_file("/tree/data.ltd", b"exact".to_vec())
        .expect("seed exact input");
    world
        .set_memory_file("/tree/data.ltd.tex", b"fallback".to_vec())
        .expect("seed extension fallback input");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexInputSearchPath::new("/job", [PathBuf::from("/tree")]);

    let content = search
        .read(&mut universe.input_open_context(), "data.ltd")
        .expect("resolve exact extension first");

    assert_eq!(content.path(), Path::new("/tree/data.ltd"));
    assert_eq!(content.bytes(), b"exact");
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

#[test]
fn area_less_font_uses_ordered_font_areas_and_records_resolved_content() {
    let mut world = World::memory();
    world
        .set_memory_file("/texlive/fonts/tfm/public/cm/cmr10.tfm", b"tfm".to_vec())
        .expect("seed searched font");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexFontSearchPath::new(
        "/job",
        [
            PathBuf::from("/texlive/fonts/tfm/local"),
            PathBuf::from("/texlive/fonts/tfm/public/cm"),
        ],
    );

    let content = search
        .read(&mut universe.input_open_context(), Path::new("cmr10"))
        .expect("resolve TFM through ordered font areas");

    assert_eq!(
        content.path(),
        Path::new("/texlive/fonts/tfm/public/cm/cmr10.tfm")
    );
    assert_eq!(universe.world().input_records().len(), 1);
    assert_eq!(universe.world().input_records()[0].path(), content.path());
    assert_eq!(universe.world().input_records()[0].hash(), content.hash());
}

#[test]
fn principal_input_area_wins_before_configured_font_areas() {
    let mut world = World::memory();
    world
        .set_memory_file("/job/cmr10.tfm", b"job".to_vec())
        .expect("seed job font");
    world
        .set_memory_file("/texlive/cmr10.tfm", b"tree".to_vec())
        .expect("seed tree font");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexFontSearchPath::new("/job", [PathBuf::from("/texlive")]);

    let content = search
        .read(&mut universe.input_open_context(), Path::new("cmr10.tfm"))
        .expect("resolve principal-area TFM first");

    assert_eq!(content.path(), Path::new("/job/cmr10.tfm"));
    assert_eq!(content.bytes(), b"job");
}

#[test]
fn explicit_font_area_does_not_fall_through_to_configured_font_areas() {
    let mut world = World::memory();
    world
        .set_memory_file("/job/sub/cmr10.tfm", b"job".to_vec())
        .expect("seed principal-area font");
    world
        .set_memory_file("/texlive/sub/cmr10.tfm", b"tree".to_vec())
        .expect("seed tree font");
    let mut universe = tex_state::Universe::with_world(world);
    let search = TexFontSearchPath::new("/job", [PathBuf::from("/texlive")]);

    let err = search
        .read(&mut universe.input_open_context(), Path::new("sub/cmr10"))
        .expect_err("explicit font area must be used as written");

    assert!(err.contains("sub/cmr10.tfm"));
    assert!(!err.contains("/job/sub/cmr10.tfm"));
    assert!(!err.contains("/texlive/sub/cmr10.tfm"));
    assert!(universe.world().input_records().is_empty());
}
