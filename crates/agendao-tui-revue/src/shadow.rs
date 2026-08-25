//! # M4.4 Shadow Comparator & Canonical Operation Normalization
//!
//! 核心原则：
//! 1. 单次副作用：Shadow 阶段绝不向服务端双发命令。
//! 2. 统一语义归一化（`CanonicalOperation`）：无论是普通 Enter、/queue、/steer、/interrupt 还是 Esc Esc，
//!    都归一化为纯净的 Canonical 表达进行比较。
//! 3. 差异多维分类（`ShadowDifference`）：涵盖请求、会话、模式、目标轮次、回执、队列及中断等多维度差异。
//! 4. 脱敏记录（`ShadowRecord`）：不持久化明文 prompt，仅保留长度、前缀摘要与 hash。
//! 5. 权限控制模式（`GatewayAuthorityMode`）：支持安全分级切换与 Kill-Switch。

use crate::command_gateway::{ClientRequestId, SessionId};
use crate::interaction_contract::TurnId;
use agendao_types::submission::{SubmissionDisposition, SubmissionMode, SubmissionRejectionReason};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 传输无关的标准操作归一化定义
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalOperation {
    Submit {
        session_id: SessionId,
        mode: SubmissionMode,
        content_hash: u64,
        content_len: usize,
        /// 调试前缀截断摘要（生产环境默认 None，杜绝凭据泄漏）
        content_preview: Option<String>,
    },
    Interrupt {
        session_id: SessionId,
        expected_turn_id: TurnId,
    },
}

impl CanonicalOperation {
    pub fn new_submit(session_id: SessionId, mode: SubmissionMode, content: &str) -> Self {
        Self::new_submit_with_masking(session_id, mode, content, false)
    }

    /// 构造提交操作（带可选的前缀截断摘要）
    pub fn new_submit_with_masking(
        session_id: SessionId,
        mode: SubmissionMode,
        content: &str,
        include_preview: bool,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();
        let content_len = content.len();
        let content_preview = if include_preview {
            let prefix: String = content.chars().take(12).collect();
            Some(format!(
                "{prefix}... (truncated summary, len={content_len})"
            ))
        } else {
            None
        };

        Self::Submit {
            session_id,
            mode,
            content_hash,
            content_len,
            content_preview,
        }
    }

    pub fn new_interrupt(session_id: SessionId, expected_turn_id: TurnId) -> Self {
        Self::Interrupt {
            session_id,
            expected_turn_id,
        }
    }

    pub fn session_id(&self) -> &str {
        match self {
            Self::Submit { session_id, .. } => session_id,
            Self::Interrupt { session_id, .. } => session_id,
        }
    }
}

/// 交互触发源
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InteractionSource {
    DefaultEnter,
    SlashQueue,
    SlashSteer,
    SlashInterrupt,
    DoubleEsc,
    LegacyAbort,
    ExplicitKeyChord,
}

/// 统一的旧路径归一化回执（用于 Shadow 比较）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizedLegacyResult {
    Started {
        session_id: SessionId,
        status: String,
    },
    Queued {
        session_id: SessionId,
    },
    Aborted {
        session_id: SessionId,
        success: bool,
    },
    Failed {
        error: String,
    },
    Unsupported,
}

/// 统一的 Gateway 归一化回执（用于 Shadow 比较）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizedGatewayResult {
    SubmitAccepted(SubmissionDisposition),
    SubmitRejected {
        reason: SubmissionRejectionReason,
        message: String,
    },
    InterruptAcknowledged {
        turn_id: TurnId,
    },
    InterruptRejected {
        reason: String,
    },
    TransportFailed {
        error: String,
    },
}

/// Shadow 差异多维分类
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowDifference {
    /// 语义完全等价
    Equivalent,
    /// 提交模式不一致 (例如 Auto vs Queue)
    RequestModeMismatch,
    /// 文本内容或 Hash 不一致
    ContentMismatch,
    /// 会话 SessionId 不一致
    SessionMismatch,
    /// 目标 Turn 不一致
    TurnTargetMismatch,
    /// 服务端接受/拒绝判定分流不一致
    DispositionMismatch,
    /// 队列元数据差异（如位置/代际变化）
    QueueMetadataMismatch,
    /// 中断状态判定不一致
    InterruptOutcomeMismatch,
    /// 旧路径不支持该语义 (例如旧 dispatch 无法直传 steer mode)
    LegacyPathUnsupported,
    /// 传输网络层异常
    TransportFailure,
}

/// Shadow 记录项（已脱敏）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowRecord {
    pub operation_id: ClientRequestId,
    pub session_id: SessionId,
    pub source: InteractionSource,
    pub canonical_operation: CanonicalOperation,
    pub legacy_result: Option<NormalizedLegacyResult>,
    pub gateway_result: Option<NormalizedGatewayResult>,
    pub difference: ShadowDifference,
    pub elapsed_ms: u64,
}

/// 网关权威切换模式（支持 Kill Switch 与渐进式放量）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GatewayAuthorityMode {
    /// 仅使用旧路径
    #[default]
    LegacyOnly,
    /// 旧路径为主执行，Gateway 做本地 Shadow 比较（零双发）
    Shadow,
    /// Gateway 作为金丝雀权威执行（特定 Session 或模式）
    GatewayCanary,
    /// Gateway 作为默认单点权威执行
    GatewayDefault,
}

/// Shadow 比较器纯函数
pub struct ShadowComparator;

impl ShadowComparator {
    /// 比较两端构造的 CanonicalOperation 是否一致
    pub fn compare_operations(
        legacy: &CanonicalOperation,
        gateway: &CanonicalOperation,
    ) -> ShadowDifference {
        if legacy.session_id() != gateway.session_id() {
            return ShadowDifference::SessionMismatch;
        }

        match (legacy, gateway) {
            (
                CanonicalOperation::Submit {
                    mode: m1,
                    content_hash: h1,
                    content_len: l1,
                    ..
                },
                CanonicalOperation::Submit {
                    mode: m2,
                    content_hash: h2,
                    content_len: l2,
                    ..
                },
            ) => {
                if m1 != m2 {
                    ShadowDifference::RequestModeMismatch
                } else if h1 != h2 || l1 != l2 {
                    ShadowDifference::ContentMismatch
                } else {
                    ShadowDifference::Equivalent
                }
            }
            (
                CanonicalOperation::Interrupt {
                    expected_turn_id: t1,
                    ..
                },
                CanonicalOperation::Interrupt {
                    expected_turn_id: t2,
                    ..
                },
            ) => {
                if t1 != t2 {
                    ShadowDifference::TurnTargetMismatch
                } else {
                    ShadowDifference::Equivalent
                }
            }
            _ => ShadowDifference::RequestModeMismatch,
        }
    }

    /// 比较两端回执（当两端都有结果或模拟结果时）
    pub fn compare_results(
        legacy: &NormalizedLegacyResult,
        gateway: &NormalizedGatewayResult,
    ) -> ShadowDifference {
        match (legacy, gateway) {
            (NormalizedLegacyResult::Unsupported, _) => ShadowDifference::LegacyPathUnsupported,
            (
                NormalizedLegacyResult::Failed { .. },
                NormalizedGatewayResult::TransportFailed { .. },
            ) => ShadowDifference::Equivalent,
            (NormalizedLegacyResult::Failed { .. }, _) => ShadowDifference::TransportFailure,
            (_, NormalizedGatewayResult::TransportFailed { .. }) => {
                ShadowDifference::TransportFailure
            }

            // Submit 比较
            (
                NormalizedLegacyResult::Started { .. },
                NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Started { .. }),
            ) => ShadowDifference::Equivalent,

            (
                NormalizedLegacyResult::Queued { .. },
                NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Queued { .. }),
            ) => ShadowDifference::Equivalent,

            (
                NormalizedLegacyResult::Started { .. },
                NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Queued { .. }),
            ) => ShadowDifference::DispositionMismatch,

            (
                NormalizedLegacyResult::Queued { .. },
                NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Started { .. }),
            ) => ShadowDifference::DispositionMismatch,

            // Interrupt 比较
            (
                NormalizedLegacyResult::Aborted { success: true, .. },
                NormalizedGatewayResult::InterruptAcknowledged { .. },
            ) => ShadowDifference::Equivalent,

            (
                NormalizedLegacyResult::Aborted { success: false, .. },
                NormalizedGatewayResult::InterruptRejected { .. },
            ) => ShadowDifference::Equivalent,

            (
                NormalizedLegacyResult::Aborted { success: true, .. },
                NormalizedGatewayResult::InterruptRejected { .. },
            ) => ShadowDifference::InterruptOutcomeMismatch,

            (
                NormalizedLegacyResult::Aborted { success: false, .. },
                NormalizedGatewayResult::InterruptAcknowledged { .. },
            ) => ShadowDifference::InterruptOutcomeMismatch,

            _ => ShadowDifference::DispositionMismatch,
        }
    }
}

/// Dark launch 观测指标汇总（支持多维度分桶与安全审计）
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ShadowMetrics {
    pub gateway_shadow_total: u64,
    pub gateway_shadow_equivalent: u64,
    pub gateway_shadow_difference: u64,
    pub gateway_shadow_transport_error: u64,
    pub gateway_shadow_legacy_unsupported: u64,

    // 按操作模式分桶
    pub submit_auto_count: u64,
    pub submit_queue_count: u64,
    pub submit_steer_count: u64,
    pub interrupt_count: u64,

    // 熔断与降级审计
    pub kill_switch_triggers: u64,
    pub fallback_to_legacy_count: u64,
}

impl ShadowMetrics {
    pub fn record(&mut self, record: &ShadowRecord) {
        self.gateway_shadow_total += 1;
        match &record.difference {
            ShadowDifference::Equivalent => self.gateway_shadow_equivalent += 1,
            ShadowDifference::TransportFailure => self.gateway_shadow_transport_error += 1,
            ShadowDifference::LegacyPathUnsupported => self.gateway_shadow_legacy_unsupported += 1,
            _ => self.gateway_shadow_difference += 1,
        }

        match &record.canonical_operation {
            CanonicalOperation::Submit { mode, .. } => match mode {
                SubmissionMode::Auto | SubmissionMode::StartTurn => self.submit_auto_count += 1,
                SubmissionMode::Queue => self.submit_queue_count += 1,
                SubmissionMode::Steer { .. } => self.submit_steer_count += 1,
            },
            CanonicalOperation::Interrupt { .. } => {
                self.interrupt_count += 1;
            }
        }
    }

    pub fn record_diff_only(&mut self, diff: &ShadowDifference) {
        self.gateway_shadow_total += 1;
        match diff {
            ShadowDifference::Equivalent => self.gateway_shadow_equivalent += 1,
            ShadowDifference::TransportFailure => self.gateway_shadow_transport_error += 1,
            ShadowDifference::LegacyPathUnsupported => self.gateway_shadow_legacy_unsupported += 1,
            _ => self.gateway_shadow_difference += 1,
        }
    }
}

/// 运行时金丝雀选择器与白名单策略
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CanarySelector {
    /// 允许走 Gateway 的 Session 白名单
    pub allowed_sessions: std::collections::HashSet<SessionId>,
    /// 允许走 Gateway 的交互源白名单
    pub allowed_sources: std::collections::HashSet<InteractionSource>,
    /// 全局金丝雀流量百分比 (0 - 100)
    pub rollout_percentage: u8,
}

impl CanarySelector {
    pub fn should_route_to_gateway(
        &self,
        session_id: &str,
        source: &InteractionSource,
        authority_mode: GatewayAuthorityMode,
    ) -> bool {
        match authority_mode {
            GatewayAuthorityMode::GatewayDefault => true,
            GatewayAuthorityMode::LegacyOnly | GatewayAuthorityMode::Shadow => false,
            GatewayAuthorityMode::GatewayCanary => {
                if self.allowed_sessions.contains(session_id) {
                    return true;
                }
                if self.allowed_sources.contains(source) {
                    return true;
                }
                if self.rollout_percentage > 0 {
                    let mut hasher = DefaultHasher::new();
                    session_id.hash(&mut hasher);
                    let score = (hasher.finish() % 100) as u8;
                    return score < self.rollout_percentage;
                }
                false
            }
        }
    }
}

/// 熔断与自动降级保护控制器 (Kill Switch & Circuit Breaker)
#[derive(Clone, Debug)]
pub struct GatewayCircuitBreaker {
    pub failure_threshold: u32,
    pub consecutive_failures: u32,
    pub tripped: bool,
}

impl Default for GatewayCircuitBreaker {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            consecutive_failures: 0,
            tripped: false,
        }
    }
}

impl GatewayCircuitBreaker {
    pub fn on_success(&mut self) {
        self.consecutive_failures = 0;
        self.tripped = false;
    }

    pub fn on_transport_failure(&mut self) -> bool {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.failure_threshold {
            self.tripped = true;
            true
        } else {
            false
        }
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped
    }

    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.tripped = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_operation_hashing_and_truncated_preview() {
        let op1 = CanonicalOperation::new_submit(
            "s1".into(),
            SubmissionMode::Auto,
            "Hello world, this is a long prompt",
        );
        let op2 = CanonicalOperation::new_submit_with_masking(
            "s1".into(),
            SubmissionMode::Auto,
            "Hello world, this is a long prompt",
            true,
        );
        let op3 =
            CanonicalOperation::new_submit("s1".into(), SubmissionMode::Auto, "Different prompt");

        if let (
            CanonicalOperation::Submit {
                content_hash: h1,
                content_len: l1,
                content_preview: p1,
                ..
            },
            CanonicalOperation::Submit {
                content_hash: h2,
                content_preview: p2,
                ..
            },
        ) = (&op1, &op2)
        {
            assert_eq!(h1, h2);
            assert_eq!(*l1, "Hello world, this is a long prompt".len());
            assert_eq!(*p1, None); // 默认不保留明文 preview
            assert_eq!(
                *p2,
                Some("Hello world,... (truncated summary, len=34)".into())
            );
        } else {
            panic!("Expected Submit variant");
        }

        assert_eq!(
            ShadowComparator::compare_operations(&op1, &op2),
            ShadowDifference::Equivalent
        );
        assert_eq!(
            ShadowComparator::compare_operations(&op1, &op3),
            ShadowDifference::ContentMismatch
        );
    }

    #[test]
    fn test_compare_operations_mode_and_turn_mismatch() {
        let op_auto = CanonicalOperation::new_submit("s1".into(), SubmissionMode::Auto, "test");
        let op_queue = CanonicalOperation::new_submit("s1".into(), SubmissionMode::Queue, "test");
        assert_eq!(
            ShadowComparator::compare_operations(&op_auto, &op_queue),
            ShadowDifference::RequestModeMismatch
        );

        let int1 = CanonicalOperation::new_interrupt("s1".into(), "t1".into());
        let int2 = CanonicalOperation::new_interrupt("s1".into(), "t2".into());
        assert_eq!(
            ShadowComparator::compare_operations(&int1, &int2),
            ShadowDifference::TurnTargetMismatch
        );
    }

    #[test]
    fn test_compare_results_submit_and_interrupt() {
        let leg_started = NormalizedLegacyResult::Started {
            session_id: "s1".into(),
            status: "running".into(),
        };
        let gw_started = NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Started {
            turn_id: "t1".into(),
            session_id: "s1".into(),
        });
        assert_eq!(
            ShadowComparator::compare_results(&leg_started, &gw_started),
            ShadowDifference::Equivalent
        );

        let gw_queued = NormalizedGatewayResult::SubmitAccepted(SubmissionDisposition::Queued {
            item_id: "q1".into(),
            session_id: "s1".into(),
            position: 1,
            queue_revision: 1,
        });
        assert_eq!(
            ShadowComparator::compare_results(&leg_started, &gw_queued),
            ShadowDifference::DispositionMismatch
        );

        let leg_abort_ok = NormalizedLegacyResult::Aborted {
            session_id: "s1".into(),
            success: true,
        };
        let gw_int_ok = NormalizedGatewayResult::InterruptAcknowledged {
            turn_id: "t1".into(),
        };
        assert_eq!(
            ShadowComparator::compare_results(&leg_abort_ok, &gw_int_ok),
            ShadowDifference::Equivalent
        );
    }

    #[test]
    fn test_multidimensional_metrics_and_canary_selector() {
        let mut m = ShadowMetrics::default();
        let rec1 = ShadowRecord {
            operation_id: "req1".into(),
            session_id: "s1".into(),
            source: InteractionSource::DefaultEnter,
            canonical_operation: CanonicalOperation::new_submit(
                "s1".into(),
                SubmissionMode::Auto,
                "test",
            ),
            legacy_result: None,
            gateway_result: None,
            difference: ShadowDifference::Equivalent,
            elapsed_ms: 5,
        };
        let rec2 = ShadowRecord {
            operation_id: "req2".into(),
            session_id: "s1".into(),
            source: InteractionSource::SlashQueue,
            canonical_operation: CanonicalOperation::new_submit(
                "s1".into(),
                SubmissionMode::Queue,
                "test",
            ),
            legacy_result: None,
            gateway_result: None,
            difference: ShadowDifference::TransportFailure,
            elapsed_ms: 10,
        };
        let rec3 = ShadowRecord {
            operation_id: "req3".into(),
            session_id: "s1".into(),
            source: InteractionSource::SlashInterrupt,
            canonical_operation: CanonicalOperation::new_interrupt("s1".into(), "t1".into()),
            legacy_result: None,
            gateway_result: None,
            difference: ShadowDifference::LegacyPathUnsupported,
            elapsed_ms: 2,
        };

        m.record(&rec1);
        m.record(&rec2);
        m.record(&rec3);

        assert_eq!(m.gateway_shadow_total, 3);
        assert_eq!(m.gateway_shadow_equivalent, 1);
        assert_eq!(m.gateway_shadow_transport_error, 1);
        assert_eq!(m.gateway_shadow_legacy_unsupported, 1);
        assert_eq!(m.submit_auto_count, 1);
        assert_eq!(m.submit_queue_count, 1);
        assert_eq!(m.interrupt_count, 1);

        // Canary Selector 测试
        let mut canary = CanarySelector::default();
        canary.allowed_sessions.insert("canary_sess".into());
        assert!(canary.should_route_to_gateway(
            "canary_sess",
            &InteractionSource::DefaultEnter,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(!canary.should_route_to_gateway(
            "other_sess",
            &InteractionSource::DefaultEnter,
            GatewayAuthorityMode::GatewayCanary
        ));

        // Circuit breaker 测试：连续 3 次失败触发熔断，成功回执自动复位
        let mut cb = GatewayCircuitBreaker::default();
        assert!(!cb.on_transport_failure());
        assert_eq!(cb.consecutive_failures, 1);
        assert!(!cb.on_transport_failure());
        assert_eq!(cb.consecutive_failures, 2);
        assert!(cb.on_transport_failure()); // 连续 3 次失败触发熔断
        assert!(cb.is_tripped());

        // 熔断状态下 on_success 必须自动复位 tripped 与 failures
        cb.on_success();
        assert!(!cb.is_tripped());
        assert_eq!(cb.consecutive_failures, 0);
    }

    #[test]
    fn test_cross_entry_canary_and_circuit_breaker_negative_scenarios() {
        let mut canary = CanarySelector::default();
        canary.allowed_sessions.insert("session_alpha".into());
        canary.allowed_sources.insert(InteractionSource::SlashQueue);

        // 1. Session 在白名单中，所有来源在 Canary 模式下走 Gateway
        assert!(canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::DefaultEnter,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::SlashQueue,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::SlashSteer,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::SlashInterrupt,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::DoubleEsc,
            GatewayAuthorityMode::GatewayCanary
        ));

        // 2. 未在白名单中的 Session，非白名单来源不得走 Gateway
        assert!(!canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::DefaultEnter,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(!canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::SlashSteer,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(!canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::SlashInterrupt,
            GatewayAuthorityMode::GatewayCanary
        ));
        assert!(!canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::DoubleEsc,
            GatewayAuthorityMode::GatewayCanary
        ));

        // 3. 来源在白名单中时（如 SlashQueue），即使 Session 不在白名单也允许走 Gateway
        assert!(canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::SlashQueue,
            GatewayAuthorityMode::GatewayCanary
        ));

        // 4. LegacyOnly 与 Shadow 模式下，无论白名单如何一律不直连 Gateway
        assert!(!canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::SlashQueue,
            GatewayAuthorityMode::LegacyOnly
        ));
        assert!(!canary.should_route_to_gateway(
            "session_alpha",
            &InteractionSource::SlashQueue,
            GatewayAuthorityMode::Shadow
        ));

        // 5. GatewayDefault 模式下一律直连 Gateway
        assert!(canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::DefaultEnter,
            GatewayAuthorityMode::GatewayDefault
        ));
        assert!(canary.should_route_to_gateway(
            "session_beta",
            &InteractionSource::DoubleEsc,
            GatewayAuthorityMode::GatewayDefault
        ));
    }
}
