use serde::{Deserialize, Serialize};
use std::fmt;

/// AI-Protocol V2 standard error codes used for cross-provider consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardErrorCode {
    InvalidRequest,
    Authentication,
    PermissionDenied,
    NotFound,
    RequestTooLarge,
    RateLimited,
    QuotaExhausted,
    ServerError,
    Overloaded,
    Timeout,
    Conflict,
    Cancelled,
    Unknown,
}

impl StandardErrorCode {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "E1001",
            Self::Authentication => "E1002",
            Self::PermissionDenied => "E1003",
            Self::NotFound => "E1004",
            Self::RequestTooLarge => "E1005",
            Self::RateLimited => "E2001",
            Self::QuotaExhausted => "E2002",
            Self::ServerError => "E3001",
            Self::Overloaded => "E3002",
            Self::Timeout => "E3003",
            Self::Conflict => "E4001",
            Self::Cancelled => "E4002",
            Self::Unknown => "E9999",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Authentication => "authentication",
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::RequestTooLarge => "request_too_large",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::ServerError => "server_error",
            Self::Overloaded => "overloaded",
            Self::Timeout => "timeout",
            Self::Conflict => "conflict",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::ServerError
                | Self::Overloaded
                | Self::Timeout
                | Self::Conflict
        )
    }

    pub fn fallbackable(&self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::RateLimited
                | Self::QuotaExhausted
                | Self::ServerError
                | Self::Overloaded
                | Self::Timeout
        )
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::InvalidRequest
            | Self::Authentication
            | Self::PermissionDenied
            | Self::NotFound
            | Self::RequestTooLarge => "client",
            Self::RateLimited | Self::QuotaExhausted => "rate",
            Self::ServerError | Self::Overloaded | Self::Timeout => "server",
            Self::Conflict | Self::Cancelled => "operational",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_provider_code(provider_code: &str) -> Option<Self> {
        let code = match provider_code {
            "invalid_request" | "invalid_request_error" | "invalid_prompt" => Self::InvalidRequest,
            "authentication" | "authentication_error" | "invalid_api_key" | "authorized_error" => {
                Self::Authentication
            }
            "permission_denied" | "permission_error" => Self::PermissionDenied,
            "not_found" | "model_not_found" => Self::NotFound,
            "request_too_large" | "context_length_exceeded" => Self::RequestTooLarge,
            "rate_limited" | "rate_limit_exceeded" => Self::RateLimited,
            "quota_exhausted" | "insufficient_quota" => Self::QuotaExhausted,
            "server_error" => Self::ServerError,
            "overloaded" | "overloaded_error" => Self::Overloaded,
            "timeout" => Self::Timeout,
            "conflict" => Self::Conflict,
            "cancelled" => Self::Cancelled,
            _ => return None,
        };
        Some(code)
    }

    pub fn from_http_status(status: u16) -> Self {
        match status {
            400 => Self::InvalidRequest,
            401 => Self::Authentication,
            403 => Self::PermissionDenied,
            404 => Self::NotFound,
            408 => Self::Timeout,
            409 => Self::Conflict,
            413 => Self::RequestTooLarge,
            429 => Self::RateLimited,
            500 => Self::ServerError,
            503 => Self::Overloaded,
            504 => Self::Timeout,
            529 => Self::Overloaded,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for StandardErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::StandardErrorCode;

    #[test]
    fn http_status_mapping_basics() {
        assert_eq!(StandardErrorCode::from_http_status(400).code(), "E1001");
        assert_eq!(StandardErrorCode::from_http_status(429).code(), "E2001");
        assert_eq!(StandardErrorCode::from_http_status(503).code(), "E3002");
    }

    #[test]
    fn provider_code_mapping_basics() {
        assert_eq!(
            StandardErrorCode::from_provider_code("context_length_exceeded")
                .unwrap()
                .code(),
            "E1005"
        );
        assert_eq!(
            StandardErrorCode::from_provider_code("insufficient_quota")
                .unwrap()
                .code(),
            "E2002"
        );
    }
}
