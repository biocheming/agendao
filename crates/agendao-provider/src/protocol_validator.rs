use crate::protocol_loader::ProtocolManifest;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolValidationError {
    #[error("protocol id is required")]
    MissingId,
    #[error("protocol version is required")]
    MissingVersion,
    #[error("endpoint.base_url is required")]
    MissingBaseUrl,
}

pub struct ProtocolValidator;

impl ProtocolValidator {
    pub fn validate(manifest: &ProtocolManifest) -> Result<(), ProtocolValidationError> {
        if manifest.id.trim().is_empty() {
            return Err(ProtocolValidationError::MissingId);
        }
        if manifest.protocol_version.trim().is_empty() {
            return Err(ProtocolValidationError::MissingVersion);
        }
        if manifest.endpoint.base_url.trim().is_empty() {
            return Err(ProtocolValidationError::MissingBaseUrl);
        }
        Ok(())
    }
}
