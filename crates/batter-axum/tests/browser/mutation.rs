use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use batter_axum::browser::{
    BrowserOrigin, FetchSitePolicy, MutationPolicy, MutationPolicyError, MutationRejection,
    RequiredHeader, RequiredHeaderError,
};

fn origin() -> BrowserOrigin {
    BrowserOrigin::https("https://example.test").unwrap()
}

fn marker() -> RequiredHeader {
    RequiredHeader::new(
        HeaderName::from_static("x-browser-request"),
        HeaderValue::from_static("1"),
    )
    .unwrap()
}

fn headers(values: &[(&'static str, &[u8])]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        headers.append(*name, HeaderValue::from_bytes(value).unwrap());
    }
    headers
}

#[test]
fn construction_requires_a_primary_signal_and_rejects_duplicate_categories() {
    let both = MutationPolicy::exact_origin(origin())
        .with_fetch_site(FetchSitePolicy::RejectCrossSite)
        .unwrap()
        .and_required_header(marker())
        .unwrap()
        .require_json()
        .unwrap();
    let valid = headers(&[
        ("origin", b"https://example.test"),
        ("sec-fetch-site", b"same-origin"),
        ("x-browser-request", b"1"),
        ("content-type", b"application/json"),
    ]);
    assert_eq!(both.check(&valid), Ok(()));
    let same_site = headers(&[
        ("origin", b"https://example.test"),
        ("sec-fetch-site", b"same-site"),
        ("x-browser-request", b"1"),
        ("content-type", b"application/json"),
    ]);
    assert_eq!(
        both.check(&same_site),
        Err(MutationRejection::FetchSiteRequired)
    );

    assert!(
        MutationPolicy::required_header(marker())
            .check(&headers(&[
                ("sec-fetch-site", b"same-origin"),
                ("x-browser-request", b"1"),
            ]))
            .is_ok()
    );
    assert!(matches!(
        MutationPolicy::exact_origin(origin()).and_exact_origin(origin()),
        Err(MutationPolicyError::OriginAlreadyConfigured)
    ));
    assert!(matches!(
        MutationPolicy::required_header(marker()).and_required_header(marker()),
        Err(MutationPolicyError::RequiredHeaderAlreadyConfigured)
    ));
    assert!(matches!(
        MutationPolicy::required_header(marker())
            .with_fetch_site(FetchSitePolicy::RequireSameOrigin),
        Err(MutationPolicyError::FetchSiteAlreadyConfigured)
    ));
    assert!(matches!(
        MutationPolicy::required_header(marker()).with_fetch_site(FetchSitePolicy::RejectCrossSite),
        Err(MutationPolicyError::FetchSiteAlreadyConfigured)
    ));
    let origin_then_marker = MutationPolicy::exact_origin(origin())
        .and_required_header(marker())
        .unwrap();
    for request in [
        headers(&[
            ("origin", b"https://example.test"),
            ("x-browser-request", b"1"),
        ]),
        headers(&[
            ("origin", b"https://example.test"),
            ("sec-fetch-site", b"same-site"),
            ("x-browser-request", b"1"),
        ]),
    ] {
        assert_eq!(
            origin_then_marker.check(&request),
            Err(MutationRejection::FetchSiteRequired)
        );
    }
    assert!(matches!(
        MutationPolicy::exact_origin(origin())
            .and_required_header(marker())
            .unwrap()
            .with_fetch_site(FetchSitePolicy::RejectCrossSite),
        Err(MutationPolicyError::FetchSiteAlreadyConfigured)
    ));
    assert!(matches!(
        MutationPolicy::exact_origin(origin())
            .with_fetch_site(FetchSitePolicy::RejectCrossSite)
            .unwrap()
            .with_fetch_site(FetchSitePolicy::RequireSameOrigin),
        Err(MutationPolicyError::FetchSiteAlreadyConfigured)
    ));
    assert!(matches!(
        MutationPolicy::exact_origin(origin())
            .require_json()
            .unwrap()
            .require_json(),
        Err(MutationPolicyError::JsonAlreadyRequired)
    ));
}

#[test]
fn custom_markers_reject_safelisted_forbidden_and_unstable_fields() {
    for name in [
        "accept",
        "accept-language",
        "content-language",
        "content-type",
        "range",
    ] {
        assert!(
            matches!(
                RequiredHeader::new(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_static("1"),
                ),
                Err(RequiredHeaderError::CorsSafelistedName)
            ),
            "{name}"
        );
    }
    for name in [
        "accept-charset",
        "accept-encoding",
        "access-control-request-headers",
        "access-control-request-method",
        "connection",
        "content-length",
        "cookie",
        "cookie2",
        "date",
        "dnt",
        "expect",
        "host",
        "keep-alive",
        "origin",
        "referer",
        "set-cookie",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "via",
        "x-http-method",
        "x-http-method-override",
        "x-method-override",
        "proxy-example",
        "sec-example",
    ] {
        assert!(
            matches!(
                RequiredHeader::new(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_static("1"),
                ),
                Err(RequiredHeaderError::BrowserControlledName)
            ),
            "{name}"
        );
    }
    assert!(matches!(
        RequiredHeader::new(
            HeaderName::from_static("x-browser-request"),
            HeaderValue::from_bytes(b"\xff").unwrap(),
        ),
        Err(RequiredHeaderError::NonVisibleAsciiValue)
    ));
    for value in [b"\t1".as_slice(), b"1\t2".as_slice(), b"1\t".as_slice()] {
        assert!(matches!(
            RequiredHeader::new(
                HeaderName::from_static("x-browser-request"),
                HeaderValue::from_bytes(value).unwrap(),
            ),
            Err(RequiredHeaderError::NonVisibleAsciiValue)
        ));
    }
    for value in [" 1", "1 "] {
        assert!(
            matches!(
                RequiredHeader::new(
                    HeaderName::from_static("x-browser-request"),
                    HeaderValue::from_str(value).unwrap(),
                ),
                Err(RequiredHeaderError::SurroundingWhitespaceValue)
            ),
            "{value:?}"
        );
    }
    assert!(
        RequiredHeader::new(
            HeaderName::from_static("x-browser-request"),
            HeaderValue::from_static("1"),
        )
        .is_ok()
    );
}

#[test]
fn browser_added_marker_cannot_bypass_automatic_same_origin_fetch_metadata() {
    let marker = RequiredHeader::new(
        HeaderName::from_static("upgrade-insecure-requests"),
        HeaderValue::from_static("1"),
    )
    .unwrap();
    let policy = MutationPolicy::required_header(marker);

    assert_eq!(
        policy.check(&headers(&[("upgrade-insecure-requests", b"1")])),
        Err(MutationRejection::FetchSiteRequired)
    );
    assert_eq!(
        policy.check(&headers(&[
            ("sec-fetch-site", b"cross-site"),
            ("upgrade-insecure-requests", b"1"),
        ])),
        Err(MutationRejection::FetchSiteCrossSite)
    );
    assert_eq!(
        policy.check(&headers(&[
            ("sec-fetch-site", b"same-origin"),
            ("upgrade-insecure-requests", b"1"),
        ])),
        Ok(())
    );
}

#[test]
fn exact_origin_requires_one_byte_exact_canonical_field() {
    let policy = MutationPolicy::exact_origin(origin());
    for (values, expected) in [
        (vec![], Err(MutationRejection::OriginMissing)),
        (vec![b"https://example.test".as_slice()], Ok(())),
        (vec![b"".as_slice()], Err(MutationRejection::OriginMismatch)),
        (
            vec![b"null".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"http://example.test".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"https://example.test:443".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"https://example.test:8443".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"https://example.test/".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"https://example.test.evil".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"https://evil.test https://example.test".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec!["https://exémple.test".as_bytes()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![
                b"https://example.test".as_slice(),
                b"https://example.test".as_slice(),
            ],
            Err(MutationRejection::OriginAmbiguous),
        ),
        (
            vec![
                b"https://example.test".as_slice(),
                b"https://evil.test".as_slice(),
            ],
            Err(MutationRejection::OriginAmbiguous),
        ),
        (
            vec![b"https://example.test, https://evil.test".as_slice()],
            Err(MutationRejection::OriginMismatch),
        ),
        (
            vec![b"\xff".as_slice()],
            Err(MutationRejection::OriginAmbiguous),
        ),
    ] {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(header::ORIGIN, HeaderValue::from_bytes(value).unwrap());
        }
        assert_eq!(policy.check(&headers), expected);
    }

    let unicode_policy =
        MutationPolicy::exact_origin(BrowserOrigin::https("https://bücher.example").unwrap());
    assert_eq!(
        unicode_policy.check(&headers(&[("origin", b"https://xn--bcher-kva.example",)])),
        Ok(())
    );
}

#[test]
fn compatible_fetch_site_rejects_only_cross_site_or_ambiguous_input() {
    let policy = MutationPolicy::exact_origin(origin())
        .with_fetch_site(FetchSitePolicy::RejectCrossSite)
        .unwrap();
    for (values, expected) in [
        (vec![], Ok(())),
        (
            vec![b"cross-site".as_slice()],
            Err(MutationRejection::FetchSiteCrossSite),
        ),
        (vec![b"same-origin".as_slice()], Ok(())),
        (vec![b"same-site".as_slice()], Ok(())),
        (vec![b"none".as_slice()], Ok(())),
        (vec![b"future-token".as_slice()], Ok(())),
        (vec![b"Cross-Site".as_slice()], Ok(())),
        (
            vec![b" future-token".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
        (
            vec![b"same-origin, same-site".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
        (
            vec![b"\xff".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
        (
            vec![b"same-origin".as_slice(), b"same-origin".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
    ] {
        let mut request = HeaderMap::new();
        request.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.test"),
        );
        for value in values {
            request.append("sec-fetch-site", HeaderValue::from_bytes(value).unwrap());
        }
        assert_eq!(policy.check(&request), expected);
    }
}

#[test]
fn strict_fetch_site_requires_one_exact_same_origin_token() {
    let policy = MutationPolicy::required_header(marker());
    for (values, expected) in [
        (vec![], Err(MutationRejection::FetchSiteRequired)),
        (vec![b"same-origin".as_slice()], Ok(())),
        (
            vec![b"cross-site".as_slice()],
            Err(MutationRejection::FetchSiteCrossSite),
        ),
        (
            vec![b"same-site".as_slice()],
            Err(MutationRejection::FetchSiteRequired),
        ),
        (
            vec![b"none".as_slice()],
            Err(MutationRejection::FetchSiteRequired),
        ),
        (
            vec![b"future-token".as_slice()],
            Err(MutationRejection::FetchSiteRequired),
        ),
        (
            vec![b"Same-Origin".as_slice()],
            Err(MutationRejection::FetchSiteRequired),
        ),
        (
            vec![b"same origin".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
        (
            vec![b"same-origin".as_slice(), b"same-origin".as_slice()],
            Err(MutationRejection::FetchSiteAmbiguous),
        ),
    ] {
        let mut request = headers(&[("x-browser-request", b"1")]);
        for value in values {
            request.append("sec-fetch-site", HeaderValue::from_bytes(value).unwrap());
        }
        assert_eq!(policy.check(&request), expected);
    }

    let origin_policy = MutationPolicy::exact_origin(origin())
        .with_fetch_site(FetchSitePolicy::RequireSameOrigin)
        .unwrap();
    for (value, expected) in [
        (None, Err(MutationRejection::FetchSiteRequired)),
        (Some("same-origin"), Ok(())),
        (Some("same-site"), Err(MutationRejection::FetchSiteRequired)),
        (Some("none"), Err(MutationRejection::FetchSiteRequired)),
    ] {
        let mut request = headers(&[("origin", b"https://example.test")]);
        if let Some(value) = value {
            request.insert("sec-fetch-site", HeaderValue::from_str(value).unwrap());
        }
        assert_eq!(origin_policy.check(&request), expected, "{value:?}");
    }
}

#[test]
fn required_header_enforces_exact_multiplicity_and_bytes() {
    let policy = MutationPolicy::required_header(marker());
    for (values, expected) in [
        (vec![], Err(MutationRejection::RequiredHeaderMissing)),
        (vec![b"1".as_slice()], Ok(())),
        (
            vec![b"".as_slice()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec![b"0".as_slice()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec![b" 1".as_slice()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec![b"1 ".as_slice()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec![b"1, 1".as_slice()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec!["é".as_bytes()],
            Err(MutationRejection::RequiredHeaderMismatch),
        ),
        (
            vec![b"\xff".as_slice()],
            Err(MutationRejection::RequiredHeaderAmbiguous),
        ),
        (
            vec![b"1".as_slice(), b"1".as_slice()],
            Err(MutationRejection::RequiredHeaderAmbiguous),
        ),
        (
            vec![b"1".as_slice(), b"0".as_slice()],
            Err(MutationRejection::RequiredHeaderAmbiguous),
        ),
    ] {
        let mut request = headers(&[("sec-fetch-site", b"same-origin")]);
        for value in values {
            request.append("x-browser-request", HeaderValue::from_bytes(value).unwrap());
        }
        assert_eq!(policy.check(&request), expected);
    }
    assert_eq!(
        policy.check(&headers(&[
            ("sec-fetch-site", b"same-origin"),
            ("x-other-request", b"1"),
        ])),
        Err(MutationRejection::RequiredHeaderMissing)
    );
}

#[test]
fn json_policy_accepts_only_exact_media_type_with_optional_parameters() {
    let policy = MutationPolicy::exact_origin(origin())
        .require_json()
        .unwrap();
    for value in [
        "application/json",
        "Application/Json",
        " application/json ",
        "application/json; charset=utf-8",
        "application/json;charset=UTF-8",
        "\tapplication/json\t;\tcharset=utf-8\t",
        "application/json;",
        "application/json ; ; charset=\"utf-8\" ;",
        "application/json; charset=utf-8; profile=compact",
        "application/json; profile=\"a,b;c\\\"d\"",
    ] {
        let request = headers(&[
            ("origin", b"https://example.test"),
            ("content-type", value.as_bytes()),
        ]);
        assert_eq!(policy.check(&request), Ok(()), "{value}");
    }
    assert_eq!(
        policy.check(&headers(&[("origin", b"https://example.test")])),
        Err(MutationRejection::JsonContentTypeMissing)
    );
    for value in [
        "",
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "application/problem+json",
        "application/vnd.example+json",
        "application/json-patch+json",
        "application/jsonx",
        "application/jsön",
        "application/json, text/plain",
        "application/json; garbage",
        "application/json; charset=",
        "application/json; =utf-8",
        "application/json; charset =utf-8",
        "application/json; charset= utf-8",
        "application/json; charset=\"unterminated",
        "application/json; charset=utf-8, text/plain",
        "application/json; charset=\"utf-8\" junk",
    ] {
        let request = headers(&[
            ("origin", b"https://example.test"),
            ("content-type", value.as_bytes()),
        ]);
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::JsonContentTypeRequired),
            "{value}"
        );
    }
    let request = headers(&[
        ("origin", b"https://example.test"),
        ("content-type", b"application/json; note=\"\xff\""),
    ]);
    assert_eq!(policy.check(&request), Ok(()));
    let request = headers(&[
        ("origin", b"https://example.test"),
        ("content-type", b"text/plain; note=\"\xff\""),
    ]);
    assert_eq!(
        policy.check(&request),
        Err(MutationRejection::JsonContentTypeAmbiguous)
    );
    for values in [
        vec![b"\xff".as_slice()],
        vec![b"application/json".as_slice(); 2],
    ] {
        let mut request = headers(&[("origin", b"https://example.test")]);
        for value in values {
            request.append(
                header::CONTENT_TYPE,
                HeaderValue::from_bytes(value).unwrap(),
            );
        }
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::JsonContentTypeAmbiguous)
        );
    }
}

#[test]
fn check_precedence_is_independent_of_builder_order() {
    let build_origin_first = || {
        MutationPolicy::exact_origin(origin())
            .with_fetch_site(FetchSitePolicy::RejectCrossSite)
            .unwrap()
            .and_required_header(marker())
            .unwrap()
            .require_json()
            .unwrap()
    };
    let build_marker_first = || {
        MutationPolicy::required_header(marker())
            .require_json()
            .unwrap()
            .and_exact_origin(origin())
            .unwrap()
    };
    for policy in [build_origin_first(), build_marker_first()] {
        let request = headers(&[
            ("origin", b"https://evil.test"),
            ("sec-fetch-site", b"cross-site"),
            ("content-type", b"text/plain"),
        ]);
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::OriginMismatch)
        );
        let request = headers(&[
            ("origin", b"https://example.test"),
            ("sec-fetch-site", b"cross-site"),
            ("content-type", b"text/plain"),
        ]);
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::FetchSiteCrossSite)
        );
        let request = headers(&[
            ("origin", b"https://example.test"),
            ("sec-fetch-site", b"same-origin"),
            ("content-type", b"text/plain"),
        ]);
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::RequiredHeaderMissing)
        );
        let request = headers(&[
            ("origin", b"https://example.test"),
            ("sec-fetch-site", b"same-origin"),
            ("x-browser-request", b"1"),
            ("content-type", b"text/plain"),
        ]);
        assert_eq!(
            policy.check(&request),
            Err(MutationRejection::JsonContentTypeRequired)
        );
    }
}

#[test]
fn policy_and_rejection_diagnostics_are_sanitized_and_application_mappable() {
    let expected = "secret-marker-value";
    let required = RequiredHeader::new(
        HeaderName::from_static("x-browser-request"),
        HeaderValue::from_static(expected),
    )
    .unwrap();
    let policy = MutationPolicy::required_header(required);
    let debug = format!("{policy:?}");
    assert!(!debug.contains(expected), "{debug}");
    let rejection = policy
        .check(&headers(&[
            ("sec-fetch-site", b"same-origin"),
            ("x-browser-request", b"observed-secret"),
        ]))
        .unwrap_err();
    for text in [format!("{rejection:?}"), rejection.to_string()] {
        assert!(!text.contains(expected), "{text}");
        assert!(!text.contains("observed-secret"), "{text}");
    }
    for (rejection, status, code) in [
        (
            MutationRejection::OriginMissing,
            StatusCode::FORBIDDEN,
            "origin_missing",
        ),
        (
            MutationRejection::OriginAmbiguous,
            StatusCode::FORBIDDEN,
            "origin_ambiguous",
        ),
        (
            MutationRejection::OriginMismatch,
            StatusCode::FORBIDDEN,
            "origin_mismatch",
        ),
        (
            MutationRejection::FetchSiteCrossSite,
            StatusCode::FORBIDDEN,
            "fetch_site_cross_site",
        ),
        (
            MutationRejection::FetchSiteRequired,
            StatusCode::FORBIDDEN,
            "fetch_site_required",
        ),
        (
            MutationRejection::FetchSiteAmbiguous,
            StatusCode::FORBIDDEN,
            "fetch_site_ambiguous",
        ),
        (
            MutationRejection::RequiredHeaderMissing,
            StatusCode::FORBIDDEN,
            "required_header_missing",
        ),
        (
            MutationRejection::RequiredHeaderAmbiguous,
            StatusCode::FORBIDDEN,
            "required_header_ambiguous",
        ),
        (
            MutationRejection::RequiredHeaderMismatch,
            StatusCode::FORBIDDEN,
            "required_header_mismatch",
        ),
        (
            MutationRejection::JsonContentTypeMissing,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_content_type_missing",
        ),
        (
            MutationRejection::JsonContentTypeAmbiguous,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_content_type_ambiguous",
        ),
        (
            MutationRejection::JsonContentTypeRequired,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_content_type_required",
        ),
    ] {
        assert_eq!(rejection.status(), status, "{rejection:?}");
        assert_eq!(rejection.code(), code, "{rejection:?}");
    }
}
