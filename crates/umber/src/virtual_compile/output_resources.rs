use std::collections::BTreeMap;
use std::fmt;

use tex_fonts::FontLayoutPolicy;

use super::{
    FileKind, OutputCapability, OutputCapabilitySet, ResourceRequest, ResourceRequestKey,
    resource_request_key, resource_sort_key,
};

/// Version of the inspectable output-resource placement plan.
pub const OUTPUT_RESOURCE_PLAN_VERSION: u8 = 1;

/// Semantic owner of a resource requirement.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourceClosureOwner {
    Engine,
    Dvi,
    Pdf,
    Html,
}

impl ResourceClosureOwner {
    const fn capability(self) -> Option<OutputCapability> {
        match self {
            Self::Engine => None,
            Self::Dvi => Some(OutputCapability::Dvi),
            Self::Pdf => Some(OutputCapability::Pdf),
            Self::Html => Some(OutputCapability::Html),
        }
    }
}

/// Why an immutable resource is retained by one closure.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourcePurpose {
    RuntimeInput,
    FormatImage,
    LayoutMetric,
    LayoutProgram,
    DviSerialization,
    PdfVirtualFont,
    PdfLocalMetric,
    PdfFontMap,
    PdfEncoding,
    PdfFontProgram,
    PdfBitmapProgram,
    HtmlLegacyMapping,
    HtmlFontTransport,
    HtmlLicense,
}

/// Acquisition semantics for one planned request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourceRequestMode {
    Prefetch,
    Probe,
    Required,
}

/// One reason a request belongs to the union.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResourceReason {
    pub owner: ResourceClosureOwner,
    pub purpose: ResourcePurpose,
}

/// A request after union deduplication. Shared objects retain every reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedResource {
    pub request: ResourceRequest,
    pub mode: ResourceRequestMode,
    pub reasons: Vec<ResourceReason>,
}

/// One semantic closure before the complete acquisition union is formed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriverResourceClosure {
    pub owner: ResourceClosureOwner,
    pub resources: Vec<PlannedResource>,
}

/// Deterministic, bounded output-neutral resource placement result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputResourcePlan {
    pub version: u8,
    pub outputs: OutputCapabilitySet,
    pub layout_policy: FontLayoutPolicy,
    pub closures: Vec<DriverResourceClosure>,
    pub union: Vec<PlannedResource>,
}

impl OutputResourcePlan {
    #[must_use]
    pub fn empty(outputs: OutputCapabilitySet, layout_policy: FontLayoutPolicy) -> Self {
        Self {
            version: OUTPUT_RESOURCE_PLAN_VERSION,
            outputs,
            layout_policy,
            closures: Vec::new(),
            union: Vec::new(),
        }
    }

    /// Builds a driver- and purpose-attributed absence for a planned request.
    #[must_use]
    pub fn missing_resource(&self, request: &ResourceRequest) -> Option<MissingOutputResource> {
        let key = resource_request_key(request);
        self.union
            .iter()
            .find(|resource| resource_request_key(&resource.request) == key)
            .map(|resource| MissingOutputResource {
                request: resource.request.clone(),
                reasons: resource.reasons.clone(),
            })
    }
}

/// Typed terminal absence retaining every responsible driver and purpose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingOutputResource {
    pub request: ResourceRequest,
    pub reasons: Vec<ResourceReason>,
}

impl fmt::Display for MissingOutputResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "missing output resource {:?} required for ",
            self.request
        )?;
        for (index, reason) in self.reasons.iter().enumerate() {
            if index != 0 {
                f.write_str(", ")?;
            }
            write!(f, "{:?}/{:?}", reason.owner, reason.purpose)?;
        }
        Ok(())
    }
}

impl std::error::Error for MissingOutputResource {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourcePlanError {
    UnsupportedHtmlVirtualFont {
        request: super::FileRequestKey,
    },
    HtmlIneligible {
        kind: FileKind,
        purpose: ResourcePurpose,
    },
    TooManyResources {
        limit: usize,
        attempted: usize,
    },
}

impl fmt::Display for ResourcePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHtmlVirtualFont { request } => write!(
                f,
                "HTML output does not support virtual font request {:?}",
                request.name()
            ),
            Self::HtmlIneligible { kind, purpose } => write!(
                f,
                "HTML resource closure cannot request {kind} for {purpose:?}"
            ),
            Self::TooManyResources { limit, attempted } => write!(
                f,
                "output resource union exceeds its resource budget ({attempted} > {limit})"
            ),
        }
    }
}

impl std::error::Error for ResourcePlanError {}

/// Mutable builder used by every native and WASM session acquisition phase.
pub(super) struct OutputResourcePlanner {
    outputs: OutputCapabilitySet,
    layout_policy: FontLayoutPolicy,
    limit: usize,
    entries: Vec<(
        ResourceClosureOwner,
        ResourcePurpose,
        ResourceRequestMode,
        ResourceRequest,
    )>,
}

impl OutputResourcePlanner {
    pub(super) fn new(
        outputs: OutputCapabilitySet,
        layout_policy: FontLayoutPolicy,
        limit: usize,
    ) -> Self {
        Self {
            outputs,
            layout_policy,
            limit,
            entries: Vec::new(),
        }
    }

    pub(super) fn add(
        &mut self,
        owner: ResourceClosureOwner,
        purpose: ResourcePurpose,
        mode: ResourceRequestMode,
        request: ResourceRequest,
    ) -> Result<(), ResourcePlanError> {
        if owner
            .capability()
            .is_some_and(|capability| !self.outputs.contains(capability))
        {
            return Ok(());
        }
        if owner == ResourceClosureOwner::Html
            && let ResourceRequest::File(file) = &request
            && !matches!(
                file.key().kind(),
                FileKind::TexInput | FileKind::FormatImage | FileKind::Tfm
            )
        {
            if file.key().kind() == FileKind::VirtualFont {
                return Err(ResourcePlanError::UnsupportedHtmlVirtualFont {
                    request: file.key().clone(),
                });
            }
            return Err(ResourcePlanError::HtmlIneligible {
                kind: file.key().kind(),
                purpose,
            });
        }
        self.entries.push((owner, purpose, mode, request));
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<OutputResourcePlan, ResourcePlanError> {
        self.entries.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
                .then_with(|| resource_sort_key(&left.3).cmp(&resource_sort_key(&right.3)))
        });

        let mut closures = BTreeMap::<ResourceClosureOwner, Vec<PlannedResource>>::new();
        let mut union = BTreeMap::<ResourceRequestKey, PlannedResource>::new();
        for (owner, purpose, mode, request) in self.entries {
            let reason = ResourceReason { owner, purpose };
            closures.entry(owner).or_default().push(PlannedResource {
                request: request.clone(),
                mode,
                reasons: vec![reason],
            });
            let entry = union
                .entry(resource_request_key(&request))
                .or_insert_with(|| PlannedResource {
                    request,
                    mode,
                    reasons: Vec::new(),
                });
            entry.mode = entry.mode.max(mode);
            if !entry.reasons.contains(&reason) {
                entry.reasons.push(reason);
            }
        }
        if union.len() > self.limit {
            return Err(ResourcePlanError::TooManyResources {
                limit: self.limit,
                attempted: union.len(),
            });
        }

        let normalize = |mut resources: Vec<PlannedResource>| {
            resources.sort_by_key(|resource| resource_sort_key(&resource.request));
            resources.dedup_by(|left, right| {
                resource_request_key(&left.request) == resource_request_key(&right.request)
                    && left.mode == right.mode
                    && left.reasons == right.reasons
            });
            resources
        };
        let closures = closures
            .into_iter()
            .map(|(owner, resources)| DriverResourceClosure {
                owner,
                resources: normalize(resources),
            })
            .collect();
        let union = normalize(union.into_values().collect());
        Ok(OutputResourcePlan {
            version: OUTPUT_RESOURCE_PLAN_VERSION,
            outputs: self.outputs,
            layout_policy: self.layout_policy,
            closures,
            union,
        })
    }
}

pub(super) fn engine_purpose(request: &ResourceRequest) -> ResourcePurpose {
    match request {
        ResourceRequest::Font(_) => ResourcePurpose::LayoutProgram,
        ResourceRequest::PkFont(_) => ResourcePurpose::PdfBitmapProgram,
        ResourceRequest::File(file) => match file.key().kind() {
            FileKind::Tfm => ResourcePurpose::LayoutMetric,
            FileKind::FormatImage => ResourcePurpose::FormatImage,
            _ => ResourcePurpose::RuntimeInput,
        },
    }
}

pub(super) fn pdf_purpose(request: &ResourceRequest) -> ResourcePurpose {
    match request {
        ResourceRequest::Font(_) => ResourcePurpose::PdfFontProgram,
        ResourceRequest::PkFont(_) => ResourcePurpose::PdfBitmapProgram,
        ResourceRequest::File(file) => match file.key().kind() {
            FileKind::VirtualFont => ResourcePurpose::PdfVirtualFont,
            FileKind::Tfm => ResourcePurpose::PdfLocalMetric,
            FileKind::PdfFontMap => ResourcePurpose::PdfFontMap,
            FileKind::PdfEncoding => ResourcePurpose::PdfEncoding,
            FileKind::PdfFontProgram => ResourcePurpose::PdfFontProgram,
            _ => ResourcePurpose::RuntimeInput,
        },
    }
}

#[cfg(test)]
mod tests;
