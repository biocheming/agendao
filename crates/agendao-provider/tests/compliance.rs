use agendao_provider::error_code::StandardErrorCode;

#[test]
fn http_status_mapping_compliance() {
    let cases: &[(u16, &str, bool, bool)] = &[
        (400, "E1001", false, false),
        (401, "E1002", false, true),
        (403, "E1003", false, false),
        (404, "E1004", false, false),
        (413, "E1005", false, false),
        (429, "E2001", true, true),
        (500, "E3001", true, true),
        (503, "E3002", true, true),
        (504, "E3003", true, true),
        (529, "E3002", true, true),
    ];

    for &(status, expected_code, expected_retryable, expected_fallbackable) in cases {
        let code = StandardErrorCode::from_http_status(status);
        assert_eq!(
            code.code(),
            expected_code,
            "status {} mapped to {}, expected {}",
            status,
            code.code(),
            expected_code
        );
        assert_eq!(code.retryable(), expected_retryable);
        assert_eq!(code.fallbackable(), expected_fallbackable);
    }
}

#[test]
fn provider_code_mapping_compliance() {
    let cases: &[(&str, &str)] = &[
        ("invalid_request_error", "E1001"),
        ("authentication_error", "E1002"),
        ("context_length_exceeded", "E1005"),
        ("rate_limit_exceeded", "E2001"),
        ("insufficient_quota", "E2002"),
        ("overloaded_error", "E3002"),
    ];

    for &(provider_code, expected_code) in cases {
        let mapped = StandardErrorCode::from_provider_code(provider_code);
        assert!(
            mapped.is_some(),
            "provider code '{}' should map to a standard code",
            provider_code
        );
        assert_eq!(
            mapped.expect("mapped code").code(),
            expected_code,
            "provider code '{}' expected {}, got {:?}",
            provider_code,
            expected_code,
            mapped
        );
    }
}
