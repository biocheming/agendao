use crate::{CapturedAssetKind, CapturedAssetSource, ModalityKind, PreflightInputPart};
use agendao_provider::{mime_to_modality, Modality};
use agendao_types::PromptPart;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultimodalPart {
    File {
        url: String,
        filename: Option<String>,
        mime: Option<String>,
        kind: CapturedAssetKind,
        byte_len: Option<usize>,
        source: CapturedAssetSource,
    },
}

impl MultimodalPart {
    pub fn file(
        url: impl Into<String>,
        filename: Option<String>,
        mime: Option<String>,
        kind: CapturedAssetKind,
        byte_len: Option<usize>,
        source: CapturedAssetSource,
    ) -> Self {
        Self::File {
            url: url.into(),
            filename,
            mime,
            kind,
            byte_len,
            source,
        }
    }

    pub fn kind(&self) -> CapturedAssetKind {
        match self {
            Self::File { kind, .. } => *kind,
        }
    }
}

pub struct SessionPartAdapter;

impl SessionPartAdapter {
    pub fn to_session_parts(parts: &[MultimodalPart]) -> Vec<PromptPart> {
        parts.iter().map(Self::to_session_part).collect()
    }

    pub fn from_session_parts(parts: &[PromptPart]) -> Vec<MultimodalPart> {
        parts.iter().filter_map(Self::from_session_part).collect()
    }

    pub fn to_preflight_parts(parts: &[MultimodalPart]) -> Vec<PreflightInputPart> {
        parts
            .iter()
            .map(|part| match part {
                MultimodalPart::File {
                    filename,
                    mime,
                    byte_len,
                    kind,
                    ..
                } => PreflightInputPart {
                    kind: Some((*kind).into()),
                    mime: mime.clone(),
                    byte_len: *byte_len,
                    label: filename.clone(),
                },
            })
            .collect()
    }

    pub fn preflight_parts_from_session_parts(parts: &[PromptPart]) -> Vec<PreflightInputPart> {
        Self::to_preflight_parts(&Self::from_session_parts(parts))
    }

    fn to_session_part(part: &MultimodalPart) -> PromptPart {
        match part {
            MultimodalPart::File {
                url,
                filename,
                mime,
                ..
            } => PromptPart::File {
                url: url.clone(),
                filename: filename.clone(),
                mime: mime.clone(),
            },
        }
    }

    fn from_session_part(part: &PromptPart) -> Option<MultimodalPart> {
        match part {
            PromptPart::File {
                url,
                filename,
                mime,
            } => Some(MultimodalPart::file(
                url.clone(),
                filename.clone(),
                mime.clone(),
                infer_asset_kind(mime.as_deref()),
                None,
                CapturedAssetSource::FileAttach,
            )),
            _ => None,
        }
    }
}

fn infer_asset_kind(mime: Option<&str>) -> CapturedAssetKind {
    let Some(mime) = mime else {
        return CapturedAssetKind::File;
    };

    match mime_to_modality(mime) {
        Some(Modality::Audio) => CapturedAssetKind::Audio,
        Some(Modality::Image) => CapturedAssetKind::Image,
        Some(Modality::Video) => CapturedAssetKind::Video,
        Some(Modality::Pdf) => CapturedAssetKind::Pdf,
        None => match resolve_kind_from_mime(mime) {
            ModalityKind::Audio => CapturedAssetKind::Audio,
            ModalityKind::Image => CapturedAssetKind::Image,
            ModalityKind::Video => CapturedAssetKind::Video,
            ModalityKind::Pdf => CapturedAssetKind::Pdf,
            ModalityKind::Text | ModalityKind::File => CapturedAssetKind::File,
        },
    }
}

fn resolve_kind_from_mime(mime: &str) -> ModalityKind {
    if mime.starts_with("audio/") {
        ModalityKind::Audio
    } else if mime.starts_with("image/") {
        ModalityKind::Image
    } else if mime.starts_with("video/") {
        ModalityKind::Video
    } else if mime == "application/pdf" {
        ModalityKind::Pdf
    } else if mime.starts_with("text/") {
        ModalityKind::Text
    } else {
        ModalityKind::File
    }
}
