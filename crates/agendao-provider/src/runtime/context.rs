#[derive(Debug, Clone)]
pub enum ProtocolSource {
    BuiltinAdapter { npm: String },
    Manifest { path: String, version: String },
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub protocol_source: ProtocolSource,
    pub provider_id: String,
    pub created_at: std::time::Instant,
}
