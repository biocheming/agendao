#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapturedAssetKind {
    Audio,
    Image,
    File,
    Video,
    Pdf,
}

impl CapturedAssetKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Image => "image",
            Self::File => "file",
            Self::Video => "video",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapturedAssetSource {
    VoiceCapture,
    FileAttach,
    BrowserUpload,
    Clipboard,
}
