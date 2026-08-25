//! M5 keyboard capability contract. Physical modifier support is tri-state:
//! unknown must fail closed (never guessed as submit/steer).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CsiuCapabilities {
    pub ctrl_enter: CapabilityState,
    pub shift_enter: CapabilityState,
}

/// Result of probing a terminal for CSI-u keyboard reporting.
///
/// A probe is deliberately separate from the capabilities it produces: an
/// unavailable terminal is a known negative, while malformed/unknown replies
/// must remain fail-closed and are therefore represented as `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsiuProbeResult {
    Supported(CsiuCapabilities),
    Unavailable,
    Malformed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEnvironment {
    Tmux,
    Xterm,
    Other,
}

/// Parse the small CSI-u response subset we need for modifier capability.
///
/// Kitty-style replies have the shape `ESC [ ? <flags> u`. A positive flag
/// means the protocol is active; `?0u` is an explicit unavailable response.
/// Anything else is intentionally treated as malformed/unknown rather than
/// guessed as supported.
pub fn parse_csiu_response(response: &str) -> CsiuProbeResult {
    let bytes = response.as_bytes();
    if bytes.len() < 5 || bytes[0] != 0x1b || bytes[1] != b'[' || bytes[2] != b'?' {
        return CsiuProbeResult::Unknown;
    }
    if bytes.last() != Some(&b'u') {
        return CsiuProbeResult::Malformed;
    }
    let flags = &bytes[3..bytes.len() - 1];
    if flags.is_empty() || !flags.iter().all(u8::is_ascii_digit) {
        return CsiuProbeResult::Malformed;
    }
    match std::str::from_utf8(flags)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    {
        Some(0) => CsiuProbeResult::Unavailable,
        Some(_) => CsiuProbeResult::Supported(CsiuCapabilities {
            ctrl_enter: CapabilityState::Supported,
            shift_enter: CapabilityState::Supported,
        }),
        None => CsiuProbeResult::Malformed,
    }
}

/// Convert a probe result into the fail-closed runtime capability state.
pub fn capabilities_from_probe(probe: CsiuProbeResult) -> CsiuCapabilities {
    match probe {
        CsiuProbeResult::Supported(capabilities) => capabilities,
        CsiuProbeResult::Unavailable => CsiuCapabilities {
            ctrl_enter: CapabilityState::Unsupported,
            shift_enter: CapabilityState::Unsupported,
        },
        CsiuProbeResult::Malformed | CsiuProbeResult::Unknown => CsiuCapabilities::default(),
    }
}

/// tmux and ordinary xterm commonly rewrite/drop modified Enter sequences;
/// without a confirmed CSI-u reply we therefore keep both capabilities
/// `Unknown` and rely on Alt+Enter or slash commands as the safe fallback.
pub fn capabilities_for_environment(environment: TerminalEnvironment) -> CsiuCapabilities {
    match environment {
        TerminalEnvironment::Tmux | TerminalEnvironment::Xterm | TerminalEnvironment::Other => {
            CsiuCapabilities::default()
        }
    }
}

impl Default for CsiuCapabilities {
    fn default() -> Self {
        Self {
            ctrl_enter: CapabilityState::Unknown,
            shift_enter: CapabilityState::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnterAction {
    Submit,
    InsertNewline,
    Ignore,
}

pub fn decide_enter(alt: bool, ctrl: bool, shift: bool, caps: CsiuCapabilities) -> EnterAction {
    if alt {
        return EnterAction::InsertNewline;
    }
    if ctrl {
        return match caps.ctrl_enter {
            CapabilityState::Supported => EnterAction::InsertNewline,
            CapabilityState::Unsupported | CapabilityState::Unknown => EnterAction::Ignore,
        };
    }
    if shift {
        return match caps.shift_enter {
            CapabilityState::Supported => EnterAction::InsertNewline,
            CapabilityState::Unsupported | CapabilityState::Unknown => EnterAction::Ignore,
        };
    }
    EnterAction::Submit
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alt_is_reliable_fallback() {
        assert_eq!(
            decide_enter(true, false, false, CsiuCapabilities::default()),
            EnterAction::InsertNewline
        );
    }
    #[test]
    fn unknown_never_submits_or_steers() {
        let c = CsiuCapabilities::default();
        assert_eq!(decide_enter(false, true, false, c), EnterAction::Ignore);
        assert_eq!(decide_enter(false, false, true, c), EnterAction::Ignore);
    }
    #[test]
    fn supported_modifier_inserts_newline() {
        let c = CsiuCapabilities {
            ctrl_enter: CapabilityState::Supported,
            shift_enter: CapabilityState::Supported,
        };
        assert_eq!(
            decide_enter(false, true, false, c),
            EnterAction::InsertNewline
        );
    }
    #[test]
    fn unsupported_modifier_is_ignored() {
        let c = CsiuCapabilities {
            ctrl_enter: CapabilityState::Unsupported,
            shift_enter: CapabilityState::Unsupported,
        };
        assert_eq!(decide_enter(false, true, false, c), EnterAction::Ignore);
    }
    #[test]
    fn bare_enter_submits() {
        assert_eq!(
            decide_enter(false, false, false, CsiuCapabilities::default()),
            EnterAction::Submit
        );
    }

    #[test]
    fn csiu_supported_response_enables_modifiers() {
        let probe = parse_csiu_response("\u{1b}[?1u");
        assert!(matches!(probe, CsiuProbeResult::Supported(_)));
        let caps = capabilities_from_probe(probe);
        assert_eq!(caps.ctrl_enter, CapabilityState::Supported);
        assert_eq!(caps.shift_enter, CapabilityState::Supported);
    }

    #[test]
    fn csiu_unavailable_is_known_negative() {
        let caps = capabilities_from_probe(parse_csiu_response("\u{1b}[?0u"));
        assert_eq!(caps.ctrl_enter, CapabilityState::Unsupported);
        assert_eq!(caps.shift_enter, CapabilityState::Unsupported);
    }

    #[test]
    fn malformed_and_unknown_fail_closed() {
        for response in ["\u{1b}[?u", "garbage", "\u{1b}[?1x"] {
            let caps = capabilities_from_probe(parse_csiu_response(response));
            assert_eq!(caps, CsiuCapabilities::default());
        }
    }

    #[test]
    fn tmux_and_xterm_use_safe_fallback() {
        for environment in [TerminalEnvironment::Tmux, TerminalEnvironment::Xterm] {
            assert_eq!(
                capabilities_for_environment(environment),
                CsiuCapabilities::default()
            );
            assert_eq!(
                decide_enter(
                    false,
                    true,
                    false,
                    capabilities_for_environment(environment)
                ),
                EnterAction::Ignore
            );
        }
    }
}
