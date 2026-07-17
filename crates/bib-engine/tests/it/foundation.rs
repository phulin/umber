use std::sync::Arc;

use bib_engine::{
    BibConfigurationBuilder, BibOptionsBuilder, BibResultBuilder, CompatibilityVersion,
    GeneratedFile, OutputFormat, OutputRequest, ProcessedBibliographyBuilder, VirtualPath,
};

#[test]
fn public_result_is_detached_and_preserves_output_order() {
    let configuration =
        BibConfigurationBuilder::new(CompatibilityVersion::BIBER_2_22_BETA).freeze();
    let document = Arc::new(ProcessedBibliographyBuilder::new(configuration).freeze());
    let first_path = VirtualPath::user("main.bbl").expect("valid output path");
    let second_path = VirtualPath::user("main.blg").expect("valid output path");
    let mut result = BibResultBuilder::new(document);
    result
        .file(GeneratedFile::new(
            first_path,
            Arc::<[u8]>::from(&b"bbl"[..]),
        ))
        .expect("unique path");
    result
        .file(GeneratedFile::new(
            second_path,
            Arc::<[u8]>::from(&b"log"[..]),
        ))
        .expect("unique path");
    let result = result.freeze();
    assert_eq!(
        result
            .files()
            .map(|file| file.path().as_str())
            .collect::<Vec<_>>(),
        ["/job/main.bbl", "/job/main.blg"]
    );
    assert_eq!(result.stats().generated_bytes(), 6);
}

#[test]
fn public_options_reject_duplicate_output_bindings() {
    let path = VirtualPath::user("main.bbl").expect("valid output path");
    let request = OutputRequest::new(path.clone(), OutputFormat::Bbl);
    let mut options = BibOptionsBuilder::new();
    options
        .output(request.clone())
        .expect("first path is unique");
    assert!(options.output(request).is_err());
}
