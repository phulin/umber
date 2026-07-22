use super::*;
use crate::{FileRequest, FileRequestKey};

fn file(kind: FileKind, name: &str) -> ResourceRequest {
    ResourceRequest::File(FileRequest::new(
        FileRequestKey::new(kind, name).expect("request key"),
        name,
    ))
}

fn font(name: &str) -> ResourceRequest {
    ResourceRequest::Font(tex_fonts::FontRequest {
        key: tex_fonts::FontRequestKey::new(
            name,
            0,
            tex_fonts::VariationSelection::default(),
            tex_fonts::FontFeaturePolicy::default(),
        )
        .expect("font request key"),
        accepted_containers: tex_fonts::AcceptedFontContainers::WASM,
        purposes: tex_fonts::FontPurposes::HTML,
    })
}

fn outputs(mask: u8) -> OutputCapabilitySet {
    let mut set = if mask & 1 != 0 {
        OutputCapabilitySet::DVI
    } else if mask & 2 != 0 {
        OutputCapabilitySet::PDF
    } else {
        OutputCapabilitySet::HTML
    };
    if mask & 2 != 0 {
        set = set.with(OutputCapability::Pdf);
    }
    if mask & 4 != 0 {
        set = set.with(OutputCapability::Html);
    }
    set
}

#[test]
fn all_output_combinations_and_layout_policies_have_exact_closures() {
    for policy in [
        FontLayoutPolicy::ClassicTfmExact,
        FontLayoutPolicy::OpenTypePreferred,
    ] {
        for mask in 1..=7 {
            let selected = outputs(mask);
            let mut planner = OutputResourcePlanner::new(selected, policy, 16);
            planner
                .add(
                    ResourceClosureOwner::Engine,
                    ResourcePurpose::LayoutMetric,
                    ResourceRequestMode::Required,
                    file(FileKind::Tfm, "shared.tfm"),
                )
                .expect("engine metric placement");
            planner
                .add(
                    ResourceClosureOwner::Dvi,
                    ResourcePurpose::DviSerialization,
                    ResourceRequestMode::Required,
                    file(FileKind::Tfm, "shared.tfm"),
                )
                .expect("DVI metric placement");
            planner
                .add(
                    ResourceClosureOwner::Pdf,
                    ResourcePurpose::PdfVirtualFont,
                    ResourceRequestMode::Probe,
                    file(FileKind::VirtualFont, "shared.vf"),
                )
                .expect("PDF VF placement");
            planner
                .add(
                    ResourceClosureOwner::Pdf,
                    ResourcePurpose::PdfFontMap,
                    ResourceRequestMode::Required,
                    file(FileKind::PdfFontMap, "pdftex.map"),
                )
                .expect("PDF map placement");
            planner
                .add(
                    ResourceClosureOwner::Html,
                    ResourcePurpose::HtmlFontTransport,
                    ResourceRequestMode::Required,
                    font("curated/cmu-serif-roman"),
                )
                .expect("HTML transport placement");
            let plan = planner.finish().expect("output resource plan");

            assert_eq!(plan.version, OUTPUT_RESOURCE_PLAN_VERSION);
            assert_eq!(plan.layout_policy, policy);
            let owners = plan
                .closures
                .iter()
                .map(|closure| closure.owner)
                .collect::<Vec<_>>();
            assert!(owners.contains(&ResourceClosureOwner::Engine));
            assert_eq!(owners.contains(&ResourceClosureOwner::Dvi), mask & 1 != 0);
            assert_eq!(owners.contains(&ResourceClosureOwner::Pdf), mask & 2 != 0);
            assert_eq!(owners.contains(&ResourceClosureOwner::Html), mask & 4 != 0);
            let expected = 1 + usize::from(mask & 2 != 0) * 2 + usize::from(mask & 4 != 0);
            assert_eq!(plan.union.len(), expected);
            assert_eq!(
                plan.union
                    .iter()
                    .filter(|resource| matches!(resource.request, ResourceRequest::File(ref file) if file.key().kind() == FileKind::Tfm))
                    .count(),
                1,
                "shared engine/DVI metrics must be acquired once"
            );
        }
    }
}

#[test]
fn html_rejects_pdf_and_dvi_only_resource_classes() {
    for kind in [
        FileKind::VirtualFont,
        FileKind::PdfFontMap,
        FileKind::PdfEncoding,
        FileKind::PdfFontProgram,
    ] {
        let mut planner = OutputResourcePlanner::new(
            OutputCapabilitySet::HTML,
            FontLayoutPolicy::ClassicTfmExact,
            8,
        );
        let error = planner
            .add(
                ResourceClosureOwner::Html,
                ResourcePurpose::HtmlFontTransport,
                ResourceRequestMode::Required,
                file(kind, "forbidden"),
            )
            .expect_err("HTML must reject legacy driver files");
        assert!(
            matches!(error, ResourcePlanError::HtmlIneligible { kind: actual, .. } if actual == kind)
        );
    }
}

#[test]
fn mixed_union_is_deterministic_deduplicated_and_reasoned() {
    let selected = OutputCapabilitySet::DVI
        .with(OutputCapability::Pdf)
        .with(OutputCapability::Html);
    let mut planner = OutputResourcePlanner::new(selected, FontLayoutPolicy::OpenTypePreferred, 4);
    for (owner, purpose) in [
        (
            ResourceClosureOwner::Html,
            ResourcePurpose::HtmlFontTransport,
        ),
        (ResourceClosureOwner::Engine, ResourcePurpose::LayoutMetric),
        (ResourceClosureOwner::Dvi, ResourcePurpose::DviSerialization),
    ] {
        planner
            .add(
                owner,
                purpose,
                ResourceRequestMode::Required,
                file(FileKind::Tfm, "cmr10.tfm"),
            )
            .expect("mixed placement");
    }
    let plan = planner.finish().expect("mixed plan");
    assert_eq!(plan.union.len(), 1);
    assert_eq!(
        plan.union[0].reasons,
        vec![
            ResourceReason {
                owner: ResourceClosureOwner::Engine,
                purpose: ResourcePurpose::LayoutMetric,
            },
            ResourceReason {
                owner: ResourceClosureOwner::Dvi,
                purpose: ResourcePurpose::DviSerialization,
            },
            ResourceReason {
                owner: ResourceClosureOwner::Html,
                purpose: ResourcePurpose::HtmlFontTransport,
            },
        ]
    );
    let missing = plan
        .missing_resource(&file(FileKind::Tfm, "cmr10.tfm"))
        .expect("planned absence");
    assert!(missing.to_string().contains("Engine/LayoutMetric"));
    assert!(missing.to_string().contains("Html/HtmlFontTransport"));
}

#[test]
fn resource_budget_applies_after_union_deduplication() {
    let mut planner = OutputResourcePlanner::new(
        OutputCapabilitySet::PDF,
        FontLayoutPolicy::ClassicTfmExact,
        1,
    );
    for name in ["one.tfm", "two.tfm"] {
        planner
            .add(
                ResourceClosureOwner::Engine,
                ResourcePurpose::LayoutMetric,
                ResourceRequestMode::Required,
                file(FileKind::Tfm, name),
            )
            .expect("budget fixture placement");
    }
    assert_eq!(
        planner.finish().expect_err("union must exceed budget"),
        ResourcePlanError::TooManyResources {
            limit: 1,
            attempted: 2,
        }
    );
}
