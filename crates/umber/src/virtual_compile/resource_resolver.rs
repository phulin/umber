use std::fmt;

use super::{NeedResources, ResourceRequest, ResourceResponse};

/// One provider's answer. Absence is scoped to this provider and never becomes
/// a session binding until the composite has exhausted every provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderResponse {
    Resolved(ResourceResponse),
    Miss(ResourceRequest),
}

/// A synchronous host adapter behind the output-neutral resource protocol.
/// Async bindings run the same provider sequence outside the engine and feed
/// the resulting response batch through `provide_resources`.
pub trait TypedResourceProvider {
    fn name(&self) -> &str;

    fn resolve(
        &mut self,
        requests: &[ResourceRequest],
        probes: &[ResourceRequest],
    ) -> Result<Vec<ProviderResponse>, ProviderFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    provider: String,
    message: String,
}

impl ProviderFailure {
    #[must_use]
    pub fn new(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "resource provider {} failed: {}",
            self.provider, self.message
        )
    }
}

impl std::error::Error for ProviderFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositeResolverError {
    Cancelled,
    Provider(ProviderFailure),
    UnexpectedResponse { provider: String },
    DuplicateResponse { provider: String },
}

impl fmt::Display for CompositeResolverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("resource resolution was cancelled"),
            Self::Provider(error) => error.fmt(formatter),
            Self::UnexpectedResponse { provider } => {
                write!(
                    formatter,
                    "resource provider {provider} returned an unexpected response"
                )
            }
            Self::DuplicateResponse { provider } => {
                write!(
                    formatter,
                    "resource provider {provider} returned a duplicate response"
                )
            }
        }
    }
}

impl std::error::Error for CompositeResolverError {}

/// Ordered provider composition. Caches remain implementation details of a
/// provider: they may supply bytes only after that provider selected the exact
/// request key.
pub struct CompositeResourceResolver {
    providers: Vec<Box<dyn TypedResourceProvider>>,
}

impl CompositeResourceResolver {
    #[must_use]
    pub fn new(providers: Vec<Box<dyn TypedResourceProvider>>) -> Self {
        Self { providers }
    }

    pub fn resolve(
        &mut self,
        needs: &NeedResources,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<Vec<ResourceResponse>, CompositeResolverError> {
        if is_cancelled() {
            return Err(CompositeResolverError::Cancelled);
        }
        let mut ordered = Vec::new();
        for request in needs.required.iter().chain(&needs.probes) {
            if !ordered.iter().any(|known| same_request(known, request)) {
                ordered.push(request.clone());
            }
        }
        let mut pending = ordered.clone();
        let mut accepted = Vec::<(ResourceRequest, ResourceResponse)>::new();
        for provider in &mut self.providers {
            if pending.is_empty() {
                break;
            }
            if is_cancelled() {
                return Err(CompositeResolverError::Cancelled);
            }
            let probes = pending
                .iter()
                .filter(|request| {
                    needs
                        .probes
                        .iter()
                        .any(|probe| same_request(request, probe))
                })
                .cloned()
                .collect::<Vec<_>>();
            let required = pending
                .iter()
                .filter(|request| {
                    !needs
                        .probes
                        .iter()
                        .any(|probe| same_request(request, probe))
                })
                .cloned()
                .collect::<Vec<_>>();
            let answers = provider
                .resolve(&required, &probes)
                .map_err(CompositeResolverError::Provider)?;
            if is_cancelled() {
                return Err(CompositeResolverError::Cancelled);
            }
            let mut seen = Vec::<ResourceRequest>::new();
            for answer in answers {
                let request = match &answer {
                    ProviderResponse::Resolved(response) => pending
                        .iter()
                        .find(|request| response_matches_request(response, request))
                        .cloned()
                        .ok_or_else(|| CompositeResolverError::UnexpectedResponse {
                            provider: provider.name().to_owned(),
                        })?,
                    ProviderResponse::Miss(request) => request.clone(),
                };
                if seen.iter().any(|known| same_request(known, &request)) {
                    return Err(CompositeResolverError::DuplicateResponse {
                        provider: provider.name().to_owned(),
                    });
                }
                if !pending.iter().any(|known| same_request(known, &request)) {
                    return Err(CompositeResolverError::UnexpectedResponse {
                        provider: provider.name().to_owned(),
                    });
                }
                seen.push(request.clone());
                if let ProviderResponse::Resolved(response) = answer {
                    pending.retain(|known| !same_request(known, &request));
                    accepted.push((request, response));
                }
            }
        }
        Ok(ordered
            .into_iter()
            .map(|request| {
                accepted
                    .iter()
                    .find(|(known, _)| same_request(known, &request))
                    .map_or_else(|| unavailable(request), |(_, response)| response.clone())
            })
            .collect())
    }
}

fn same_request(left: &ResourceRequest, right: &ResourceRequest) -> bool {
    match (left, right) {
        (ResourceRequest::File(left), ResourceRequest::File(right)) => left.key() == right.key(),
        (ResourceRequest::Font(left), ResourceRequest::Font(right)) => left.key == right.key,
        (ResourceRequest::PkFont(left), ResourceRequest::PkFont(right)) => left == right,
        _ => false,
    }
}

fn response_matches_request(response: &ResourceResponse, request: &ResourceRequest) -> bool {
    match (response, request) {
        (ResourceResponse::File(file), ResourceRequest::File(request)) => {
            file.request == *request.key()
        }
        (ResourceResponse::FileUnavailable(key), ResourceRequest::File(request)) => {
            key == request.key()
        }
        (ResourceResponse::Font(font), ResourceRequest::Font(request)) => {
            font.request == request.key
        }
        (ResourceResponse::FontUnavailable(key), ResourceRequest::Font(request)) => {
            key == &request.key
        }
        (ResourceResponse::PkFont(font), ResourceRequest::PkFont(request)) => {
            &font.request == request
        }
        (ResourceResponse::PkFontUnavailable(key), ResourceRequest::PkFont(request)) => {
            key == request
        }
        _ => false,
    }
}

fn unavailable(request: ResourceRequest) -> ResourceResponse {
    match request {
        ResourceRequest::File(request) => ResourceResponse::FileUnavailable(request.key().clone()),
        ResourceRequest::Font(request) => ResourceResponse::FontUnavailable(request.key),
        ResourceRequest::PkFont(request) => ResourceResponse::PkFontUnavailable(request),
    }
}

#[cfg(test)]
mod tests;
