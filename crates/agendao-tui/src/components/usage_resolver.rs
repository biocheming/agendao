use agendao_types::{SessionUsage, SessionUsageBooks};

/// Which authority level produced the resolved usage values.
///
/// This exists to satisfy AgenDao Article 10 (可观测性权利):
/// for any displayed usage number we must be able to answer
/// "who wrote it, who reads it, who displays it, who has the final say."
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageAuthority {
    /// Resolved from [`SessionUsageBooks::workflow_cumulative`] —
    /// the canonical server-side telemetry source.
    Books,
    /// Resolved from [`SessionUsage`] — partial server-side telemetry
    /// (e.g. when usage books have not been populated yet).
    Usage,
    /// Resolved from a client-side message fold —
    /// used only as a last resort when no server telemetry is available.
    #[default]
    MessageFold,
}

/// Client-side fallback values computed by folding individual message
/// token/cost fields. Passed to [`resolve_usage`] as the last-resort
/// authority level.
#[derive(Clone, Debug, Default)]
pub struct MessageFoldUsage {
    pub total_cost: f64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_miss_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Usage values resolved from a **single** authority level.
///
/// All fields in this struct come from the same source; this guarantees
/// the sidebar never mixes, for example, cost from [`UsageAuthority::Books`]
/// with cache tokens from [`UsageAuthority::MessageFold`].
#[derive(Clone, Debug, Default)]
pub struct ResolvedUsage {
    /// Which authority level produced all the fields below.
    /// Reserved for B2 return-flow strip and debug observability.
    #[allow(dead_code)]
    pub authority: UsageAuthority,
    pub total_cost: f64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_miss_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Resolve all cumulative usage fields from the highest-available authority level.
///
/// Priority (with liveness check):
/// 1. [`SessionUsageBooks::workflow_cumulative`] — canonical source, **only if populated**
/// 2. [`SessionUsage`] — partial server telemetry
/// 3. `message_fold` — client-side estimate (last resort)
///
/// "Populated" means at least one cumulative token field is non-zero.
/// The server atomically writes both `usage_books` and `usage` via
/// `apply_session_telemetry_snapshot`, but `workflow_cumulative` may
/// arrive as a zero-valued default before real data is available.
/// In that case we fall through to `usage` rather than displaying zeros.
///
/// All returned fields come from the **same** level; the three
/// independent fallback chains that previously existed in the sidebar
/// are replaced by this single entry-point.
pub fn resolve_usage(
    usage_books: Option<&SessionUsageBooks>,
    usage: Option<&SessionUsage>,
    message_fold: Option<&MessageFoldUsage>,
) -> ResolvedUsage {
    if let Some(books) = usage_books {
        if is_cumulative_populated(&books.workflow_cumulative) {
            let cumulative = &books.workflow_cumulative;
            return ResolvedUsage {
                authority: UsageAuthority::Books,
                total_cost: cumulative.total_cost,
                total_tokens: cumulative.total_tokens(),
                cache_read_tokens: cumulative.cache_read_tokens,
                cache_miss_tokens: cumulative.cache_miss_tokens,
                cache_write_tokens: cumulative.cache_write_tokens,
            };
        }
    }

    if let Some(usage) = usage {
        return ResolvedUsage {
            authority: UsageAuthority::Usage,
            total_cost: usage.total_cost,
            total_tokens: total_session_tokens(usage),
            cache_read_tokens: usage.cache_read_tokens,
            cache_miss_tokens: usage.cache_miss_tokens,
            cache_write_tokens: usage.cache_write_tokens,
        };
    }

    if let Some(fold) = message_fold {
        return ResolvedUsage {
            authority: UsageAuthority::MessageFold,
            total_cost: fold.total_cost,
            total_tokens: fold.total_tokens,
            cache_read_tokens: fold.cache_read_tokens,
            cache_miss_tokens: fold.cache_miss_tokens,
            cache_write_tokens: fold.cache_write_tokens,
        };
    }

    ResolvedUsage {
        authority: UsageAuthority::MessageFold,
        ..Default::default()
    }
}

/// Returns `true` when `cumulative` carries real data (at least one
/// token field is positive).  A zero-valued default that was written
/// atomically with `session_usage` should not be treated as authoritative.
fn is_cumulative_populated(cumulative: &agendao_types::WorkflowUsageSummary) -> bool {
    cumulative.input_tokens > 0
        || cumulative.output_tokens > 0
        || cumulative.reasoning_tokens > 0
        || cumulative.cache_read_tokens > 0
        || cumulative.cache_miss_tokens > 0
        || cumulative.cache_write_tokens > 0
        || cumulative.total_cost > 0.0
}

fn total_session_tokens(usage: &SessionUsage) -> u64 {
    usage.input_tokens + usage.output_tokens + usage.reasoning_tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::WorkflowUsageSummary;

    fn workflow_summary(
        cost: f64,
        input: u64,
        output: u64,
        reasoning: u64,
    ) -> WorkflowUsageSummary {
        WorkflowUsageSummary {
            total_cost: cost,
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: reasoning,
            cache_read_tokens: 0,
            cache_miss_tokens: 0,
            cache_write_tokens: 0,
        }
    }

    fn usage_with(cost: f64, input: u64, output: u64) -> SessionUsage {
        SessionUsage {
            total_cost: cost,
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_falls_back_to_message_fold_when_no_telemetry() {
        let fold = MessageFoldUsage {
            total_cost: 3.5,
            total_tokens: 320,
            cache_read_tokens: 15,
            cache_miss_tokens: 8,
            cache_write_tokens: 3,
        };
        let resolved = resolve_usage(None, None, Some(&fold));
        assert_eq!(resolved.authority, UsageAuthority::MessageFold);
        assert_eq!(resolved.total_cost, 3.5);
    }

    #[test]
    fn resolve_prefers_usage_over_message_fold() {
        let usage = usage_with(42.0, 100, 200);
        let fold = MessageFoldUsage {
            total_cost: 99.0,
            ..Default::default()
        };
        let resolved = resolve_usage(None, Some(&usage), Some(&fold));
        assert_eq!(resolved.authority, UsageAuthority::Usage);
        assert_eq!(resolved.total_cost, 42.0);
    }

    #[test]
    fn resolve_prefers_books_over_everything() {
        let books = SessionUsageBooks {
            workflow_cumulative: workflow_summary(7.0, 10, 20, 5),
            ..Default::default()
        };
        let usage = usage_with(99.0, 1, 1);
        let fold = MessageFoldUsage {
            total_cost: 999.0,
            ..Default::default()
        };
        let resolved = resolve_usage(Some(&books), Some(&usage), Some(&fold));
        assert_eq!(resolved.authority, UsageAuthority::Books);
        assert_eq!(resolved.total_cost, 7.0);
        assert_eq!(resolved.total_tokens, 10 + 20 + 5);
    }

    #[test]
    fn resolve_falls_through_empty_books_to_usage() {
        // Simulates the real scenario: server writes both books and usage,
        // but workflow_cumulative is still zero-valued (not yet populated).
        let books = SessionUsageBooks {
            workflow_cumulative: WorkflowUsageSummary::default(),
            ..Default::default()
        };
        let usage = usage_with(42.0, 100, 200);
        let fold = MessageFoldUsage {
            total_cost: 99.0,
            ..Default::default()
        };
        let resolved = resolve_usage(Some(&books), Some(&usage), Some(&fold));
        assert_eq!(
            resolved.authority,
            UsageAuthority::Usage,
            "empty books should fall through to usage, not display zeros"
        );
        assert_eq!(resolved.total_cost, 42.0);
        assert_eq!(resolved.total_tokens, 300);
    }

    #[test]
    fn resolve_returns_consistent_authority_for_all_fields() {
        let usage = SessionUsage {
            total_cost: 1.0,
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 30,
            cache_miss_tokens: 40,
            cache_write_tokens: 50,
            ..Default::default()
        };
        let resolved = resolve_usage(None, Some(&usage), None);
        assert_eq!(resolved.authority, UsageAuthority::Usage);
        assert_eq!(resolved.total_cost, 1.0);
        assert_eq!(resolved.cache_read_tokens, 30);
    }
}
