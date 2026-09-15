use batter_axum::browser::{BrowserOrigin, BrowserOriginError};

#[test]
fn https_origins_are_canonical_and_restricted_to_origin_structure() {
    for (input, expected) in [
        ("https://example.test", "https://example.test"),
        ("https://example.test/", "https://example.test"),
        ("https://EXAMPLE.test", "https://example.test"),
        ("HTTPS://example.test", "https://example.test"),
        ("https://example.test:443", "https://example.test"),
        ("https://example.test:8443", "https://example.test:8443"),
        ("https://bücher.example", "https://xn--bcher-kva.example"),
    ] {
        let origin = BrowserOrigin::https(input).unwrap();
        assert_eq!(origin.as_str(), expected, "{input}");
        assert!(origin.is_secure());
    }

    for (input, expected) in [
        ("http://example.test", BrowserOriginError::UnsupportedScheme),
        ("ftp://example.test", BrowserOriginError::UnsupportedScheme),
        ("/api", BrowserOriginError::InvalidUrl),
        ("https:///path", BrowserOriginError::MissingHost),
        (
            "https://user@example.test",
            BrowserOriginError::CredentialsNotAllowed,
        ),
        (
            "https://user:pass@example.test",
            BrowserOriginError::CredentialsNotAllowed,
        ),
        (
            "https://@example.test",
            BrowserOriginError::CredentialsNotAllowed,
        ),
        (
            "https://example.test/path",
            BrowserOriginError::PathNotAllowed,
        ),
        (
            "https://example.test/foo/..",
            BrowserOriginError::PathNotAllowed,
        ),
        ("https://example.test/.", BrowserOriginError::PathNotAllowed),
        (
            "https://example.test/%2e",
            BrowserOriginError::PathNotAllowed,
        ),
        ("https://example.test\\.", BrowserOriginError::InvalidUrl),
        (
            "HTTPS://example.test/%2e",
            BrowserOriginError::PathNotAllowed,
        ),
        ("https:/example.test/a/..", BrowserOriginError::InvalidUrl),
        ("https:example.test/a/..", BrowserOriginError::InvalidUrl),
        ("https:@example.test", BrowserOriginError::InvalidUrl),
        (
            "https:\\example.test\\a\\..",
            BrowserOriginError::InvalidUrl,
        ),
        ("HTTPS:////example.test", BrowserOriginError::MissingHost),
        (
            "https://example.test/?q=1",
            BrowserOriginError::QueryNotAllowed,
        ),
        (
            "https://example.test/#x",
            BrowserOriginError::FragmentNotAllowed,
        ),
        ("null", BrowserOriginError::InvalidUrl),
        ("", BrowserOriginError::InvalidUrl),
        (" https://example.test", BrowserOriginError::InvalidUrl),
        ("https://exa\tmple.test", BrowserOriginError::InvalidUrl),
    ] {
        assert_eq!(BrowserOrigin::https(input), Err(expected), "{input}");
    }
}

#[test]
fn loopback_http_accepts_only_explicit_local_hosts() {
    for (input, expected) in [
        ("http://localhost", "http://localhost"),
        ("http://localhost:80", "http://localhost"),
        ("http://localhost:3000/", "http://localhost:3000"),
        ("http://app.localhost", "http://app.localhost"),
        ("http://127.0.0.1", "http://127.0.0.1"),
        ("http://127.12.34.56", "http://127.12.34.56"),
        ("http://[::1]", "http://[::1]"),
    ] {
        let origin = BrowserOrigin::loopback_http(input).unwrap();
        assert_eq!(origin.as_str(), expected, "{input}");
        assert!(!origin.is_secure());
    }

    for (input, expected) in [
        (
            "http://localhost.evil.test",
            BrowserOriginError::InsecureNonLoopback,
        ),
        (
            "http://example.test",
            BrowserOriginError::InsecureNonLoopback,
        ),
        ("http://10.0.0.1", BrowserOriginError::InsecureNonLoopback),
        ("http://0.0.0.0", BrowserOriginError::InsecureNonLoopback),
        ("http://[::2]", BrowserOriginError::InsecureNonLoopback),
        (
            "http://[::ffff:127.0.0.1]",
            BrowserOriginError::InsecureNonLoopback,
        ),
        ("https://localhost", BrowserOriginError::UnsupportedScheme),
        (
            "http://user@localhost",
            BrowserOriginError::CredentialsNotAllowed,
        ),
        ("http://localhost/path", BrowserOriginError::PathNotAllowed),
        ("null", BrowserOriginError::InvalidUrl),
        ("", BrowserOriginError::InvalidUrl),
    ] {
        assert_eq!(
            BrowserOrigin::loopback_http(input),
            Err(expected),
            "{input}"
        );
    }
}

#[test]
fn origin_errors_never_reproduce_untrusted_configuration() {
    let secret = "https://user:secret@example.test";
    let error = BrowserOrigin::https(secret).unwrap_err();
    for text in [format!("{error:?}"), error.to_string()] {
        assert!(!text.contains(secret), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }
}
