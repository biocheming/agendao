use super::SchedulerBlueprint;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, thiserror::Error)]
#[error("failed to canonicalize scheduler blueprint: {0}")]
pub struct CanonicalizationError(#[from] serde_json::Error);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlueprintFingerprint([u8; 32]);

impl BlueprintFingerprint {
    pub fn from_blueprint(blueprint: &SchedulerBlueprint) -> Result<Self, CanonicalizationError> {
        // All map/set surfaces in SchedulerBlueprint use ordered collections.
        // Struct field order is fixed by the schema, so serde_json is canonical here.
        let bytes = serde_json::to_vec(blueprint)?;
        Ok(Self(Sha256::digest(bytes).into()))
    }
}

impl fmt::Display for BlueprintFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
