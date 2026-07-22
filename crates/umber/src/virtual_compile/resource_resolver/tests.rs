use std::collections::VecDeque;

use super::*;
use crate::{FileKind, FileRequest, FileRequestKey, ResolvedFile};

struct Provider {
    name: &'static str,
    answers: VecDeque<Result<Vec<ProviderResponse>, ProviderFailure>>,
}

impl TypedResourceProvider for Provider {
    fn name(&self) -> &str {
        self.name
    }

    fn resolve(
        &mut self,
        _requests: &[ResourceRequest],
        _probes: &[ResourceRequest],
    ) -> Result<Vec<ProviderResponse>, ProviderFailure> {
        self.answers.pop_front().expect("provider answer")
    }
}

fn request(name: &str, kind: FileKind) -> ResourceRequest {
    ResourceRequest::File(FileRequest::new(
        FileRequestKey::new(kind, name).expect("request key"),
        name,
    ))
}

fn resolved(request: &ResourceRequest, byte: u8) -> ProviderResponse {
    let ResourceRequest::File(request) = request else {
        panic!("file request")
    };
    ProviderResponse::Resolved(ResourceResponse::File(ResolvedFile {
        request: request.key().clone(),
        virtual_path: format!("/objects/{byte}"),
        bytes: vec![byte],
        expected_digest: None,
    }))
}

fn provider(
    name: &'static str,
    answer: Result<Vec<ProviderResponse>, ProviderFailure>,
) -> Box<dyn TypedResourceProvider> {
    Box::new(Provider {
        name,
        answers: VecDeque::from([answer]),
    })
}

#[test]
fn higher_precedence_wins_and_provider_miss_falls_through() {
    let private = request("private.tex", FileKind::TexInput);
    let hosted = request("hosted.tex", FileKind::TexInput);
    let mut resolver = CompositeResourceResolver::new(vec![
        provider(
            "private",
            Ok(vec![
                resolved(&private, 1),
                ProviderResponse::Miss(hosted.clone()),
            ]),
        ),
        provider("hosted", Ok(vec![resolved(&hosted, 2)])),
    ]);
    let responses = resolver
        .resolve(
            &NeedResources {
                required: vec![private, hosted],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            || false,
        )
        .expect("resolved");
    let bytes = responses
        .iter()
        .map(|response| match response {
            ResourceResponse::File(file) => file.bytes.clone(),
            _ => panic!("positive file"),
        })
        .collect::<Vec<_>>();
    assert_eq!(bytes, vec![vec![1], vec![2]]);
}

#[test]
fn exact_kind_identity_does_not_alias_equal_names() {
    let tex = request("cmr10", FileKind::TexInput);
    let tfm = request("cmr10", FileKind::Tfm);
    let mut resolver = CompositeResourceResolver::new(vec![provider(
        "local",
        Ok(vec![ProviderResponse::Miss(tex.clone()), resolved(&tfm, 3)]),
    )]);
    let responses = resolver
        .resolve(
            &NeedResources {
                required: vec![tex, tfm],
                probes: Vec::new(),
                prefetch_hints: Vec::new(),
            },
            || false,
        )
        .expect("resolved");
    assert!(matches!(responses[0], ResourceResponse::FileUnavailable(_)));
    assert!(matches!(&responses[1], ResourceResponse::File(file) if file.bytes == [3]));
}

#[test]
fn failure_and_cancellation_never_become_authoritative_absence() {
    let needed = request("plain.tex", FileKind::TexInput);
    let failure = ProviderFailure::new("hosted", "offline object missing");
    let mut resolver =
        CompositeResourceResolver::new(vec![provider("hosted", Err(failure.clone()))]);
    let needs = NeedResources {
        required: vec![needed],
        probes: Vec::new(),
        prefetch_hints: Vec::new(),
    };
    assert_eq!(
        resolver.resolve(&needs, || false),
        Err(CompositeResolverError::Provider(failure))
    );
    assert_eq!(
        resolver.resolve(&needs, || true),
        Err(CompositeResolverError::Cancelled)
    );
}
