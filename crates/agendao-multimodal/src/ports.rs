use std::path::{Path, PathBuf};

pub trait CapturePort {
    type Request;
    type Output;
    type Error;

    fn capture(&self, request: Self::Request) -> Result<Self::Output, Self::Error>;
}

pub trait AttachmentLoaderPort {
    type Error;

    fn load_attachment(&self, path: &Path) -> Result<PathBuf, Self::Error>;
}
