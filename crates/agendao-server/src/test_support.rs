use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static FIXTURE_SEQUENCE: AtomicU32 = AtomicU32::new(0);

pub(crate) fn target_fixture_root(name: &str) -> PathBuf {
    let configured = std::env::var("CARGO_TARGET_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            panic!("CARGO_TARGET_DIR is not set; run tests with CARGO_TARGET_DIR=../target")
        });
    let configured = PathBuf::from(configured);
    let target_root = if configured.is_absolute() {
        configured
    } else {
        workspace_root().join(configured)
    };
    let fixture = target_root
        .join("agendao-server-unit-tests")
        .join(name)
        .join(format!(
            "{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&fixture)
        .unwrap_or_else(|error| panic!("create fixture {}: {error}", fixture.display()));
    std::fs::canonicalize(&fixture)
        .unwrap_or_else(|error| panic!("canonicalize fixture {}: {error}", fixture.display()))
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|candidate| candidate.join(".cargo/config.toml").is_file())
        .unwrap_or_else(|| panic!("cannot find workspace root above {}", manifest.display()))
        .to_path_buf()
}

// ── Scripted provider: shared by scheduler contract and transport parity tests ──

/// One scripted model-call behavior.
pub(crate) enum ScriptedTurn {
    /// Return these stream events as a complete assistant turn.
    Events(Vec<agendao_provider::StreamEvent>),
    /// `chat_stream` returns this error directly.
    Fail(agendao_provider::ProviderError),
}

/// Scripted provider: answers `chat_stream` from a script, counts calls, and
/// can hang (pending stream) for cancellation-contract tests. Registering it
/// in a `ProviderRegistry` additionally requires `with_model_info` so
/// `get_model` resolves — the registry lookup path checks it.
pub(crate) struct ScriptedProvider {
    calls: std::sync::atomic::AtomicUsize,
    hang: std::sync::atomic::AtomicBool,
    script: std::sync::Mutex<std::collections::VecDeque<ScriptedTurn>>,
    model_info: std::sync::OnceLock<agendao_provider::ModelInfo>,
}

impl ScriptedProvider {
    pub(crate) fn new(script: Vec<ScriptedTurn>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            hang: std::sync::atomic::AtomicBool::new(false),
            script: std::sync::Mutex::new(script.into()),
            model_info: std::sync::OnceLock::new(),
        })
    }

    /// Give the provider a resolvable model so `ProviderRegistry` selection
    /// (`parse_model_string` + `get_model`) succeeds for `model: "<id>/<model>"`.
    pub(crate) fn with_model_info(
        self: &std::sync::Arc<Self>,
        model: agendao_provider::ModelInfo,
    ) -> std::sync::Arc<Self> {
        let _ = self.model_info.set(model);
        self.clone()
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn hang(&self) {
        self.hang
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) fn text_turn(text: &str) -> ScriptedTurn {
    ScriptedTurn::Events(vec![
        agendao_provider::StreamEvent::Start,
        agendao_provider::StreamEvent::TextStart,
        agendao_provider::StreamEvent::TextDelta(text.to_string()),
        agendao_provider::StreamEvent::TextEnd,
        agendao_provider::StreamEvent::FinishStep {
            finish_reason: Some("stop".to_string()),
            usage: agendao_provider::StreamUsage::default(),
            provider_metadata: None,
        },
    ])
}

#[async_trait::async_trait]
impl agendao_provider::Provider for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    fn name(&self) -> &str {
        "Scripted"
    }

    fn provider_profile_fingerprint(&self) -> Option<agendao_provider::ProviderProfileFingerprint> {
        None
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        self.model_info.get().cloned().into_iter().collect()
    }

    fn get_model(&self, id: &str) -> Option<&agendao_provider::ModelInfo> {
        self.model_info.get().filter(|model| model.id == id)
    }

    async fn chat(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
        Err(agendao_provider::ProviderError::InvalidRequest(
            "scripted provider only supports chat_stream".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.hang.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(Box::pin(futures::stream::pending()));
        }
        let next = self.script.lock().expect("script lock").pop_front();
        match next {
            Some(ScriptedTurn::Events(events)) => {
                Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
            }
            Some(ScriptedTurn::Fail(error)) => Err(error),
            None => Ok(Box::pin(futures::stream::iter(
                vec![Ok(agendao_provider::StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: agendao_provider::StreamUsage::default(),
                    provider_metadata: None,
                })]
                .into_iter(),
            ))),
        }
    }
}
