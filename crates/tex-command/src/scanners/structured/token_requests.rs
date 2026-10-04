//! Token ownership mapping on the genuine scanner request values.
use super::*;

/// Visits and transfers only token fields, retaining each request's semantics.
/// Mapping consumes the source request and constructs its final owned fields.
pub trait TokenRequest<T>: Sized {
    type Mapped<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T));
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> Self::Mapped<U>;
}
impl TokenRequest<AttemptTokenListId> for AttemptTokenListId {
    type Mapped<U> = U;
    fn visit_tokens(&self, f: &mut impl FnMut(&AttemptTokenListId)) {
        f(self)
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(AttemptTokenListId) -> U) -> U {
        f(self)
    }
}
impl<T, R: TokenRequest<T>> TokenRequest<T> for Option<R> {
    type Mapped<U> = Option<R::Mapped<U>>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = self {
            value.visit_tokens(f)
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> Self::Mapped<U> {
        self.map(|value| value.map_tokens(f))
    }
}
impl<T, R: TokenRequest<T>> TokenRequest<T> for Vec<R> {
    type Mapped<U> = Vec<R::Mapped<U>>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        for value in self {
            value.visit_tokens(f)
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> Self::Mapped<U> {
        self.into_iter().map(|value| value.map_tokens(f)).collect()
    }
}
impl<T, A: TokenRequest<T>, B: TokenRequest<T>> TokenRequest<T> for (A, B) {
    type Mapped<U> = (A::Mapped<U>, B::Mapped<U>);
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        self.0.visit_tokens(f);
        self.1.visit_tokens(f)
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> Self::Mapped<U> {
        (self.0.map_tokens(f), self.1.map_tokens(f))
    }
}
impl<T> TokenRequest<T> for ScannedBalancedText<T> {
    type Mapped<U> = ScannedBalancedText<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        f(&self.tokens);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> ScannedBalancedText<U> {
        ScannedBalancedText {
            tokens: f(self.tokens),
            provenance: self.provenance,
        }
    }
}
impl<T> TokenRequest<T> for PdfActionIdentifier<T> {
    type Mapped<U> = PdfActionIdentifier<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfActionIdentifier::Name(field0) => {
                f(field0);
            }
            PdfActionIdentifier::Number(_field0) => {}
            PdfActionIdentifier::Raw(field0) => {
                f(field0);
            }
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfActionIdentifier<U> {
        match self {
            PdfActionIdentifier::Name(field0) => PdfActionIdentifier::Name(f(field0)),
            PdfActionIdentifier::Number(field0) => PdfActionIdentifier::Number(field0),
            PdfActionIdentifier::Raw(field0) => PdfActionIdentifier::Raw(f(field0)),
        }
    }
}
impl<T> TokenRequest<T> for PdfActionTarget<T> {
    type Mapped<U> = PdfActionTarget<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfActionTarget::Page { number: _, view } => {
                f(view);
            }
            PdfActionTarget::Destination(field0) => {
                field0.visit_tokens(f);
            }
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfActionTarget<U> {
        match self {
            PdfActionTarget::Page { number, view } => PdfActionTarget::Page {
                number,
                view: f(view),
            },
            PdfActionTarget::Destination(field0) => {
                PdfActionTarget::Destination(field0.map_tokens(f))
            }
        }
    }
}
impl<T> TokenRequest<T> for PdfActionDestination<T> {
    type Mapped<U> = PdfActionDestination<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = &self.file {
            f(value);
        }
        if let Some(value) = &self.structure {
            value.visit_tokens(f);
        }
        self.target.visit_tokens(f);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfActionDestination<U> {
        PdfActionDestination {
            file: self.file.map(&mut *f),
            structure: self.structure.map(|value| value.map_tokens(f)),
            target: self.target.map_tokens(f),
            window: self.window,
        }
    }
}
impl<T> TokenRequest<T> for PdfActionSpec<T> {
    type Mapped<U> = PdfActionSpec<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfActionSpec::User(field0) => {
                f(field0);
            }
            PdfActionSpec::GoTo(field0) => {
                field0.visit_tokens(f);
            }
            PdfActionSpec::Thread(field0) => {
                field0.visit_tokens(f);
            }
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfActionSpec<U> {
        match self {
            PdfActionSpec::User(field0) => PdfActionSpec::User(f(field0)),
            PdfActionSpec::GoTo(field0) => PdfActionSpec::GoTo(field0.map_tokens(f)),
            PdfActionSpec::Thread(field0) => PdfActionSpec::Thread(field0.map_tokens(f)),
        }
    }
}
impl<T> TokenRequest<T> for PdfImageRequest<T> {
    type Mapped<U> = PdfImageRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = &self.attr {
            f(value);
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfImageRequest<U> {
        PdfImageRequest {
            name: self.name,
            width: self.width,
            height: self.height,
            depth: self.depth,
            page: self.page,
            color_space_object: self.color_space_object,
            page_box: self.page_box,
            page_box_explicit: self.page_box_explicit,
            resolution: self.resolution,
            attr: self.attr.map(&mut *f),
        }
    }
}
impl<T> TokenRequest<T> for PdfGraphicsRequest<T> {
    type Mapped<U> = PdfGraphicsRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfGraphicsRequest::Literal {
                mode: _,
                deferred: _,
                text,
            } => {
                text.visit_tokens(f);
            }
            PdfGraphicsRequest::SetMatrix { text } => {
                text.visit_tokens(f);
            }
            PdfGraphicsRequest::Save => {}
            PdfGraphicsRequest::Restore => {}
            PdfGraphicsRequest::ColorStack { id: _, action } => {
                if let Some(value) = action {
                    value.visit_tokens(f);
                }
            }
            PdfGraphicsRequest::SavePosition => {}
            PdfGraphicsRequest::SnapReferencePoint => {}
            PdfGraphicsRequest::SnapY { glue: _ } => {}
            PdfGraphicsRequest::SnapYComp { ratio: _ } => {}
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfGraphicsRequest<U> {
        match self {
            PdfGraphicsRequest::Literal {
                mode,
                deferred,
                text,
            } => PdfGraphicsRequest::Literal {
                mode,
                deferred,
                text: text.map_tokens(f),
            },
            PdfGraphicsRequest::SetMatrix { text } => PdfGraphicsRequest::SetMatrix {
                text: text.map_tokens(f),
            },
            PdfGraphicsRequest::Save => PdfGraphicsRequest::Save,
            PdfGraphicsRequest::Restore => PdfGraphicsRequest::Restore,
            PdfGraphicsRequest::ColorStack { id, action } => PdfGraphicsRequest::ColorStack {
                id,
                action: action.map(|value| value.map_tokens(f)),
            },
            PdfGraphicsRequest::SavePosition => PdfGraphicsRequest::SavePosition,
            PdfGraphicsRequest::SnapReferencePoint => PdfGraphicsRequest::SnapReferencePoint,
            PdfGraphicsRequest::SnapY { glue } => PdfGraphicsRequest::SnapY { glue },
            PdfGraphicsRequest::SnapYComp { ratio } => PdfGraphicsRequest::SnapYComp { ratio },
        }
    }
}
impl<T> TokenRequest<T> for PdfColorStackActionRequest<T> {
    type Mapped<U> = PdfColorStackActionRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfColorStackActionRequest::Set(field0) => {
                field0.visit_tokens(f);
            }
            PdfColorStackActionRequest::Push(field0) => {
                field0.visit_tokens(f);
            }
            PdfColorStackActionRequest::Pop => {}
            PdfColorStackActionRequest::Current => {}
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfColorStackActionRequest<U> {
        match self {
            PdfColorStackActionRequest::Set(field0) => {
                PdfColorStackActionRequest::Set(field0.map_tokens(f))
            }
            PdfColorStackActionRequest::Push(field0) => {
                PdfColorStackActionRequest::Push(field0.map_tokens(f))
            }
            PdfColorStackActionRequest::Pop => PdfColorStackActionRequest::Pop,
            PdfColorStackActionRequest::Current => PdfColorStackActionRequest::Current,
        }
    }
}
impl<T> TokenRequest<T> for PdfObjectRequest<T> {
    type Mapped<U> = PdfObjectRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfObjectRequest::Reserve => {}
            PdfObjectRequest::Define {
                use_object: _,
                stream: _,
                stream_attr,
                file: _,
                data,
            } => {
                if let Some(value) = stream_attr {
                    value.visit_tokens(f);
                }
                data.visit_tokens(f);
            }
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfObjectRequest<U> {
        match self {
            PdfObjectRequest::Reserve => PdfObjectRequest::Reserve,
            PdfObjectRequest::Define {
                use_object,
                stream,
                stream_attr,
                file,
                data,
            } => PdfObjectRequest::Define {
                use_object,
                stream,
                stream_attr: stream_attr.map(|value| value.map_tokens(f)),
                file,
                data: data.map_tokens(f),
            },
        }
    }
}
impl<T> TokenRequest<T> for PdfFormRequest<T> {
    type Mapped<U> = PdfFormRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfFormRequest::Create {
                attr,
                resources,
                box_register: _,
            } => {
                if let Some(value) = attr {
                    value.visit_tokens(f);
                }
                if let Some(value) = resources {
                    value.visit_tokens(f);
                }
            }
            PdfFormRequest::Reference { object: _ } => {}
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfFormRequest<U> {
        match self {
            PdfFormRequest::Create {
                attr,
                resources,
                box_register,
            } => PdfFormRequest::Create {
                attr: attr.map(|value| value.map_tokens(f)),
                resources: resources.map(|value| value.map_tokens(f)),
                box_register,
            },
            PdfFormRequest::Reference { object } => PdfFormRequest::Reference { object },
        }
    }
}
impl<T> TokenRequest<T> for PdfDocumentFragmentRequest<T> {
    type Mapped<U> = PdfDocumentFragmentRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        self.text.visit_tokens(f);
        if let Some(value) = &self.open_action {
            value.visit_tokens(f);
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfDocumentFragmentRequest<U> {
        PdfDocumentFragmentRequest {
            kind: self.kind,
            text: self.text.map_tokens(f),
            open_action: self.open_action.map(|value| value.map_tokens(f)),
        }
    }
}
impl<T> TokenRequest<T> for PdfNavigationRequest<T> {
    type Mapped<U> = PdfNavigationRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfNavigationRequest::Annotation(field0) => {
                field0.visit_tokens(f);
            }
            PdfNavigationRequest::StartLink(field0) => {
                field0.visit_tokens(f);
            }
            PdfNavigationRequest::EndLink => {}
            PdfNavigationRequest::Outline(field0) => {
                field0.visit_tokens(f);
            }
            PdfNavigationRequest::Destination(field0) => {
                field0.visit_tokens(f);
            }
            PdfNavigationRequest::Thread(field0) => {
                field0.visit_tokens(f);
            }
            PdfNavigationRequest::EndThread => {}
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfNavigationRequest<U> {
        match self {
            PdfNavigationRequest::Annotation(field0) => {
                PdfNavigationRequest::Annotation(field0.map_tokens(f))
            }
            PdfNavigationRequest::StartLink(field0) => {
                PdfNavigationRequest::StartLink(field0.map_tokens(f))
            }
            PdfNavigationRequest::EndLink => PdfNavigationRequest::EndLink,
            PdfNavigationRequest::Outline(field0) => {
                PdfNavigationRequest::Outline(field0.map_tokens(f))
            }
            PdfNavigationRequest::Destination(field0) => {
                PdfNavigationRequest::Destination(field0.map_tokens(f))
            }
            PdfNavigationRequest::Thread(field0) => {
                PdfNavigationRequest::Thread(field0.map_tokens(f))
            }
            PdfNavigationRequest::EndThread => PdfNavigationRequest::EndThread,
        }
    }
}
impl<T> TokenRequest<T> for PdfAnnotationRequest<T> {
    type Mapped<U> = PdfAnnotationRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        match self {
            PdfAnnotationRequest::Reserve => {}
            PdfAnnotationRequest::Define {
                use_object: _,
                dimensions: _,
                entries,
            } => {
                entries.visit_tokens(f);
            }
        }
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfAnnotationRequest<U> {
        match self {
            PdfAnnotationRequest::Reserve => PdfAnnotationRequest::Reserve,
            PdfAnnotationRequest::Define {
                use_object,
                dimensions,
                entries,
            } => PdfAnnotationRequest::Define {
                use_object,
                dimensions,
                entries: entries.map_tokens(f),
            },
        }
    }
}
impl<T> TokenRequest<T> for PdfStartLinkRequest<T> {
    type Mapped<U> = PdfStartLinkRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = &self.attributes {
            value.visit_tokens(f);
        }
        self.action.visit_tokens(f);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfStartLinkRequest<U> {
        PdfStartLinkRequest {
            dimensions: self.dimensions,
            attributes: self.attributes.map(|value| value.map_tokens(f)),
            action: self.action.map_tokens(f),
        }
    }
}
impl<T> TokenRequest<T> for PdfOutlineRequest<T> {
    type Mapped<U> = PdfOutlineRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = &self.attributes {
            value.visit_tokens(f);
        }
        self.action.visit_tokens(f);
        self.title.visit_tokens(f);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfOutlineRequest<U> {
        PdfOutlineRequest {
            attributes: self.attributes.map(|value| value.map_tokens(f)),
            action: self.action.map_tokens(f),
            count: self.count,
            title: self.title.map_tokens(f),
        }
    }
}
impl<T> TokenRequest<T> for PdfDestinationRequest<T> {
    type Mapped<U> = PdfDestinationRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        self.identifier.visit_tokens(f);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfDestinationRequest<U> {
        PdfDestinationRequest {
            structure: self.structure,
            identifier: self.identifier.map_tokens(f),
            kind: self.kind,
        }
    }
}
impl<T> TokenRequest<T> for PdfThreadRequest<T> {
    type Mapped<U> = PdfThreadRequest<U>;
    fn visit_tokens(&self, f: &mut impl FnMut(&T)) {
        if let Some(value) = &self.attributes {
            value.visit_tokens(f);
        }
        self.identifier.visit_tokens(f);
    }
    fn map_tokens<U>(self, f: &mut impl FnMut(T) -> U) -> PdfThreadRequest<U> {
        PdfThreadRequest {
            dimensions: self.dimensions,
            attributes: self.attributes.map(|value| value.map_tokens(f)),
            identifier: self.identifier.map_tokens(f),
            running: self.running,
        }
    }
}
