pub(crate) mod events;
pub(crate) mod frontend_projection;
pub(crate) mod frontend_subscription;
pub mod local_frontend;
pub(crate) mod memory;
pub(crate) mod projection_authority;
pub(crate) mod recheck_loop;
pub(crate) mod steering;
pub(crate) mod task_ledger;
pub(crate) mod task_ledger_reducer;
pub mod task_ledger_stall;
pub(crate) mod telemetry;

use std::sync::Arc;

use agendao_provider::Provider;
use agendao_session::{PartType, Session, SessionMessage};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ModelPricing {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_write_per_million: f64,
}

impl ModelPricing {
    pub(crate) fn new(
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: Option<f64>,
        cache_write_per_million: Option<f64>,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_read_per_million: cache_read_per_million.unwrap_or(input_per_million),
            cache_write_per_million: cache_write_per_million.unwrap_or(input_per_million),
        }
    }

    pub(crate) fn from_model_info(info: &agendao_provider::ModelInfo) -> Self {
        Self::new(
            info.cost_per_million_input,
            info.cost_per_million_output,
            info.cost_per_million_cache_read,
            info.cost_per_million_cache_write,
        )
    }

    pub(crate) fn compute(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_miss_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let uncached_input_tokens = if cache_miss_tokens > 0 {
            cache_miss_tokens
        } else {
            input_tokens
        };
        let input_cost = self.input_per_million * uncached_input_tokens as f64 / 1_000_000.0;
        let output_cost = self.output_per_million * output_tokens as f64 / 1_000_000.0;
        let cache_read_cost = self.cache_read_per_million * cache_read_tokens as f64 / 1_000_000.0;
        let cache_write_cost =
            self.cache_write_per_million * cache_write_tokens as f64 / 1_000_000.0;
        input_cost + output_cost + cache_read_cost + cache_write_cost
    }
}

pub fn assistant_visible_text(message: &SessionMessage) -> String {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::Text { text, ignored, .. } if ignored != &Some(true) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    agendao_session::sanitize_display_text(&text)
}

pub(crate) async fn ensure_default_session_title(
    session: &mut Session,
    provider: Arc<dyn Provider>,
    model_id: &str,
) {
    let Some((_, fallback)) = agendao_session::compose_session_title_source(session) else {
        return;
    };

    let old_session_title = session.record().title.clone();
    if !session.allows_auto_title_regeneration() && old_session_title.trim() != fallback.trim() {
        return;
    }

    let generated_title =
        agendao_session::generate_session_title_for_session(session, provider, model_id).await;
    if !generated_title.trim().is_empty() {
        tracing::info!(
            session_id = %session.record().id,
            old_title = %old_session_title,
            new_title = %generated_title,
            "session title refined by model"
        );
        session.set_title(generated_title);
    }
}
