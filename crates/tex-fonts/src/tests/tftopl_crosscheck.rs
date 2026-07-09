#![allow(clippy::disallowed_methods)] // host-side reference harness

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate as tex_fonts;
use anyhow::{Context, Result, bail};
use refexec::RefTftopl;
use test_support::{
    live_reference_enabled,
    pl::{PlCharacter, PlExtensibleRecipe, PlFont, PlLigCommand, PlLigLabel, PlNumber},
};
use tex_arith::{FontSizeSpec, Scaled, tfm_fix_word_to_scaled};
use tex_fonts::tfm::Character;
use tex_fonts::{CharacterTag, FontParameterKind, LigKernAction, TfmFont};

const CORPUS_FONTS: &[&str] = &["cmr10", "cmmi10", "cmsy10", "cmex10", "cmtt10"];
const VARIANTS: &[(&str, FontSizeSpec)] = &[
    ("design", FontSizeSpec::Design),
    (
        "at_11pt",
        FontSizeSpec::At(Scaled::from_raw(11 * Scaled::UNITY)),
    ),
    ("scaled_1200", FontSizeSpec::Scale(1200)),
];

#[test]
fn tftopl_crosscheck_computer_modern_corpus() -> Result<()> {
    if !live_reference_enabled() {
        eprintln!("skipping tftopl corpus cross-check: set UMBER_LIVE_REF=1");
        return Ok(());
    }

    let font_paths = match corpus_font_paths(&third_party_fonts_root()) {
        Ok(paths) => paths,
        Err(skip) => {
            eprintln!("{skip}");
            return Ok(());
        }
    };
    let tftopl = match RefTftopl::locate() {
        Ok(tftopl) => tftopl,
        Err(error) => {
            eprintln!("skipping tftopl corpus cross-check: {error:#}");
            return Ok(());
        }
    };

    for (name, path) in font_paths {
        crosscheck_font(&tftopl, &name, &path)?;
    }

    Ok(())
}

#[test]
fn tftopl_crosscheck_edge_ligkern_fixtures() -> Result<()> {
    if !live_reference_enabled() {
        eprintln!("skipping tftopl edge-fixture cross-check: set UMBER_LIVE_REF=1");
        return Ok(());
    }

    let tftopl = match RefTftopl::locate() {
        Ok(tftopl) => tftopl,
        Err(error) => {
            eprintln!("skipping tftopl edge-fixture cross-check: {error:#}");
            return Ok(());
        }
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge");

    for name in ["boundary-char", "ptmr8g-longjump"] {
        crosscheck_font(&tftopl, name, &root.join(format!("{name}.tfm")))?;
    }

    Ok(())
}

#[test]
fn corpus_font_paths_reports_missing_fonts_as_skip() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let skip = corpus_font_paths(temp_dir.path()).expect_err("empty corpus should skip");

    assert!(skip.contains("skipping tftopl corpus cross-check"));
    assert!(skip.contains("scripts/fetch-font-corpus.sh"));
    assert!(skip.contains("cmr10.tfm"));
}

fn crosscheck_font(tftopl: &RefTftopl, name: &str, path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let pl = test_support::pl::parse_font(&tftopl.to_pl(path)?)
        .with_context(|| format!("failed to parse tftopl PL for {name}"))?;

    for (variant_name, size_spec) in VARIANTS {
        let font = TfmFont::parse_with_size(&bytes, *size_spec)
            .with_context(|| format!("failed to parse {name} variant {variant_name}"))?;
        compare_font(name, variant_name, &font, &pl)?;
    }

    Ok(())
}

fn compare_font(name: &str, variant: &str, font: &TfmFont, pl: &PlFont) -> Result<()> {
    let context = || format!("{name} {variant}");

    if variant == "design" {
        assert_eq!(
            Some(font.header.checksum),
            pl.checksum,
            "{} checksum",
            context()
        );
        assert_eq!(
            font.header.design_size.raw(),
            pl.design_size
                .as_ref()
                .context("PL is missing DESIGNSIZE")?
                .to_scaled_points()?,
            "{} design size",
            context()
        );
        assert_eq!(
            font.right_boundary_char,
            pl.boundary_char,
            "{} boundary char",
            context()
        );
    }

    compare_characters(name, variant, font, pl)?;
    compare_lig_kerns(name, variant, font, pl)?;
    compare_parameters(name, variant, font, pl)?;

    Ok(())
}

fn compare_characters(name: &str, variant: &str, font: &TfmFont, pl: &PlFont) -> Result<()> {
    let actual_codes: Vec<u8> = font.characters.iter().flatten().map(|ch| ch.code).collect();
    let expected_codes: Vec<u8> = pl.characters.keys().copied().collect();
    assert_eq!(
        actual_codes, expected_codes,
        "{name} {variant} character set"
    );

    for character in font.characters.iter().flatten() {
        let pl_character = pl
            .characters
            .get(&character.code)
            .with_context(|| format!("{name} {variant} missing PL character {}", character.code))?;
        assert_eq!(
            character.width.raw(),
            pl_metric(pl_character.width.as_ref(), font.font_size)?,
            "{name} {variant} char {} width",
            character.code
        );
        assert_eq!(
            character.height.raw(),
            pl_metric(pl_character.height.as_ref(), font.font_size)?,
            "{name} {variant} char {} height",
            character.code
        );
        assert_eq!(
            character.depth.raw(),
            pl_metric(pl_character.depth.as_ref(), font.font_size)?,
            "{name} {variant} char {} depth",
            character.code
        );
        assert_eq!(
            character.italic_correction.raw(),
            pl_metric(pl_character.italic_correction.as_ref(), font.font_size)?,
            "{name} {variant} char {} italic",
            character.code
        );
        compare_character_tag(name, variant, font, character, pl_character)?;
    }

    Ok(())
}

fn compare_character_tag(
    name: &str,
    variant: &str,
    font: &TfmFont,
    character: &Character,
    pl_character: &PlCharacter,
) -> Result<()> {
    match (
        character.tag,
        pl_character.next_larger,
        pl_character.extensible_recipe,
    ) {
        (CharacterTag::None | CharacterTag::LigKern { .. }, None, None) => {}
        (CharacterTag::NextLarger(actual), Some(expected), None) => {
            assert_eq!(actual, expected, "{name} {variant} next larger");
        }
        (CharacterTag::Extensible(index), None, Some(expected)) => {
            assert_eq!(
                font.extensible_recipes.get(usize::from(index)).copied(),
                Some(pl_recipe_to_tfm(expected)),
                "{name} {variant} char {} extensible recipe",
                character.code
            );
        }
        _ => bail!(
            "{name} {variant} char {} tag mismatch: {:?} vs PL next={:?} recipe={:?}",
            character.code,
            character.tag,
            pl_character.next_larger,
            pl_character.extensible_recipe
        ),
    }
    Ok(())
}

fn compare_lig_kerns(name: &str, variant: &str, font: &TfmFont, pl: &PlFont) -> Result<()> {
    let actual = actual_lig_tables(font)?;
    let expected = expected_lig_tables(pl, font.font_size)?;
    assert_eq!(actual, expected, "{name} {variant} lig/kern tables");
    Ok(())
}

fn compare_parameters(name: &str, variant: &str, font: &TfmFont, pl: &PlFont) -> Result<()> {
    assert_eq!(
        font.parameters.values.len(),
        pl.parameters.len(),
        "{name} {variant} fontdimen count"
    );

    for (actual, expected) in font.parameters.values.iter().zip(&pl.parameters) {
        let expected_value = match actual.kind {
            FontParameterKind::SlantRatio => {
                i32::from_be_bytes(expected.value.to_fix_word_bytes()?) / 16
            }
            FontParameterKind::Dimension => {
                tfm_fix_word_to_scaled(expected.value.to_fix_word_bytes()?, font.font_size)?.raw()
            }
        };
        assert_eq!(
            actual.value.raw(),
            expected_value,
            "{name} {variant} fontdimen{} {}",
            actual.number,
            expected.name
        );
    }

    Ok(())
}

fn actual_lig_tables(font: &TfmFont) -> Result<BTreeMap<PlLigLabel, Vec<ExpectedLigCommand>>> {
    let mut tables = BTreeMap::new();

    if let Some(start) = font.left_boundary_program {
        tables.insert(
            PlLigLabel::Boundary,
            actual_lig_commands(font, usize::from(start))?,
        );
    }

    for character in font.characters.iter().flatten() {
        if let CharacterTag::LigKern { start_index, .. } = character.tag {
            tables.insert(
                PlLigLabel::Character(character.code),
                actual_lig_commands(font, usize::from(start_index))?,
            );
        }
    }

    Ok(tables)
}

fn actual_lig_commands(font: &TfmFont, mut index: usize) -> Result<Vec<ExpectedLigCommand>> {
    let mut commands = Vec::new();
    loop {
        let step = font
            .lig_kern_program
            .get(index)
            .with_context(|| format!("lig/kern start {index} is out of bounds"))?;
        match step.action {
            Some(LigKernAction::Ligature(ligature)) => {
                commands.push(ExpectedLigCommand::Ligature {
                    right: step.next_char,
                    replacement: ligature.replacement,
                    delete_current: ligature.deletes.current,
                    delete_next: ligature.deletes.next,
                    pass_over: ligature.pass_over,
                });
            }
            Some(LigKernAction::Kern(kern)) => {
                commands.push(ExpectedLigCommand::Kern {
                    right: step.next_char,
                    amount: kern.amount.raw(),
                });
            }
            None => {}
        }
        if step.skip_byte >= 128 {
            break;
        }
        index += usize::from(step.skip_byte) + 1;
    }
    Ok(commands)
}

fn expected_lig_tables(
    pl: &PlFont,
    font_size: Scaled,
) -> Result<BTreeMap<PlLigLabel, Vec<ExpectedLigCommand>>> {
    let mut tables = BTreeMap::new();

    for table in &pl.lig_tables {
        let commands = table
            .commands
            .iter()
            .map(|command| expected_lig_command(command, font_size))
            .collect::<Result<Vec<_>>>()?;
        for label in &table.labels {
            tables.insert(*label, commands.clone());
        }
    }

    Ok(tables)
}

fn expected_lig_command(command: &PlLigCommand, font_size: Scaled) -> Result<ExpectedLigCommand> {
    Ok(match command {
        PlLigCommand::Ligature(ligature) => ExpectedLigCommand::Ligature {
            right: ligature.right,
            replacement: ligature.replacement,
            delete_current: ligature.delete_current,
            delete_next: ligature.delete_next,
            pass_over: ligature.pass_over,
        },
        PlLigCommand::Kern { right, amount } => ExpectedLigCommand::Kern {
            right: *right,
            amount: tfm_fix_word_to_scaled(amount.to_fix_word_bytes()?, font_size)?.raw(),
        },
    })
}

fn pl_metric(number: Option<&PlNumber>, font_size: Scaled) -> Result<i32> {
    match number {
        Some(number) => Ok(tfm_fix_word_to_scaled(number.to_fix_word_bytes()?, font_size)?.raw()),
        None => Ok(0),
    }
}

fn corpus_font_paths(root: &Path) -> std::result::Result<Vec<(String, PathBuf)>, String> {
    let mut missing = Vec::new();
    let mut paths = Vec::new();

    for name in CORPUS_FONTS {
        let path = root.join(format!("{name}.tfm"));
        if path.is_file() {
            paths.push(((*name).to_owned(), path));
        } else {
            missing.push(format!("{name}.tfm"));
        }
    }

    if missing.is_empty() {
        Ok(paths)
    } else {
        Err(format!(
            "skipping tftopl corpus cross-check: missing {} in {}; run scripts/fetch-font-corpus.sh",
            missing.join(", "),
            root.display()
        ))
    }
}

fn third_party_fonts_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/fonts")
}

fn pl_recipe_to_tfm(value: PlExtensibleRecipe) -> tex_fonts::ExtensibleRecipe {
    tex_fonts::ExtensibleRecipe {
        top: value.top,
        middle: value.middle,
        bottom: value.bottom,
        repeated: value.repeated,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExpectedLigCommand {
    Ligature {
        right: u8,
        replacement: u8,
        delete_current: bool,
        delete_next: bool,
        pass_over: u8,
    },
    Kern {
        right: u8,
        amount: i32,
    },
}
