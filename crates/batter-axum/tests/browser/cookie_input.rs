use axum::http::{HeaderMap, HeaderValue, header};
use batter_axum::browser::{
    CookieHeaderError, CookieHeaderPolicy, CookieName, CookieValue, read_cookie,
};

fn headers(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(header::COOKIE, HeaderValue::from_str(value).unwrap());
    }
    headers
}

fn read(
    headers: &HeaderMap,
    policy: CookieHeaderPolicy,
) -> Result<Option<&str>, CookieHeaderError> {
    let name = CookieName::new("session").unwrap();
    read_cookie(headers, &name, policy).map(|value| value.map(CookieValue::as_str))
}

#[test]
fn cookie_names_use_the_exact_case_sensitive_token_subset() {
    for name in [
        "session",
        "studio_session",
        "identity-session",
        "__Host-session",
        "A",
        "a1",
        "!#$%&'*+-.^_`|~",
    ] {
        assert_eq!(CookieName::new(name).unwrap().as_str(), name);
    }
    for name in [
        "",
        " session",
        "ses sion",
        "session\t",
        "session\n",
        "session\r",
        "session\0",
        "sessión",
        "name=value",
        "name;value",
        "name,value",
        "name/value",
        "name\\value",
        "name\"value",
        "name(value",
        "name<value",
        "name>value",
        "name@value",
        "name:value",
        "name[value",
        "name]value",
        "name?value",
        "name{value",
        "name}value",
        "name\u{7f}",
    ] {
        let error = CookieName::new(name).unwrap_err();
        for text in [format!("{error:?}"), error.to_string()] {
            if !name.is_empty() {
                assert!(!text.contains(name), "{name:?}: {text}");
            }
        }
    }
    assert_ne!(
        CookieName::new("Session").unwrap(),
        CookieName::new("session").unwrap()
    );
}

#[test]
fn opaque_values_are_unquoted_unchanged_and_redacted() {
    let long = "a".repeat(8 * 1024);
    for value in [
        "",
        "abc_DEF-123",
        "YWJjZA==",
        "deadbeef",
        "DEADBEEF",
        "!#$%&'()*+-./:<=>?@[]^_`{|}~",
        long.as_str(),
    ] {
        let value = CookieValue::new(value).unwrap();
        if !value.as_str().is_empty() {
            assert!(!format!("{value:?}").contains(value.as_str()));
        }
    }
    for value in [
        " leading",
        "trailing ",
        "tab\tvalue",
        "line\nvalue",
        "return\rvalue",
        "nul\0value",
        "quoted\"value",
        "comma,value",
        "semi;value",
        "slash\\value",
        "delete\u{7f}",
        "unicodé",
        "\"quoted\"",
    ] {
        let error = CookieValue::new(value).unwrap_err();
        for text in [format!("{error:?}"), error.to_string()] {
            assert!(!text.contains(value), "{value:?}: {text}");
        }
    }
}

#[test]
fn every_cookie_field_is_scanned_and_target_values_remain_opaque() {
    for (fields, expected) in [
        (vec![], None),
        (vec!["session=abc"], Some("abc")),
        (vec!["other=x; session=abc"], Some("abc")),
        (vec!["session=abc; other=x"], Some("abc")),
        (vec!["Session=abc"], None),
        (vec!["session="], Some("")),
        (vec!["session=a=b"], Some("a=b")),
        (vec![" session = abc "], Some("abc")),
        (vec!["other=x", "session=abc"], Some("abc")),
        (vec!["session=abc; Session=def"], Some("abc")),
        (vec!["sessions=abc"], None),
        (vec!["__Host-session=abc"], None),
        (vec![";;session=abc;;"], Some("abc")),
        (vec!["session=abc%20def"], Some("abc%20def")),
    ] {
        let headers = headers(&fields);
        for policy in [CookieHeaderPolicy::Strict, CookieHeaderPolicy::TargetOnly] {
            assert_eq!(read(&headers, policy).unwrap(), expected, "{fields:?}");
        }
    }
}

#[test]
fn duplicate_or_malformed_targets_are_never_accepted() {
    for fields in [
        vec!["session=one; session=two"],
        vec!["session=one", "session=two"],
    ] {
        let headers = headers(&fields);
        for policy in [CookieHeaderPolicy::Strict, CookieHeaderPolicy::TargetOnly] {
            assert_eq!(
                read(&headers, policy),
                Err(CookieHeaderError::DuplicateTarget)
            );
        }
    }
    for field in [
        "session",
        "session=\"quoted\"",
        "session=comma,value",
        "session=tab\tvalue",
    ] {
        let headers = headers(&[field]);
        let expected = if field == "session" {
            CookieHeaderError::MalformedPair
        } else {
            CookieHeaderError::InvalidTargetValue
        };
        for policy in [CookieHeaderPolicy::Strict, CookieHeaderPolicy::TargetOnly] {
            assert_eq!(read(&headers, policy), Err(expected), "{field:?}");
        }
    }
}

#[test]
fn malformed_unrelated_pairs_obey_the_selected_policy() {
    for field in [
        "broken; session=abc",
        "other=\"bad\"; session=abc",
        "other=café; session=abc",
        "session=abc; other=café",
    ] {
        let headers = headers(&[field]);
        assert_eq!(
            read(&headers, CookieHeaderPolicy::TargetOnly),
            Ok(Some("abc"))
        );
        assert_eq!(
            read(&headers, CookieHeaderPolicy::Strict),
            Err(CookieHeaderError::MalformedPair)
        );
    }
    let headers = headers(&["broken"]);
    assert_eq!(read(&headers, CookieHeaderPolicy::TargetOnly), Ok(None));
    assert_eq!(
        read(&headers, CookieHeaderPolicy::Strict),
        Err(CookieHeaderError::MalformedPair)
    );
}

#[test]
fn non_utf8_fields_cannot_hide_a_target_before_or_after_it() {
    for non_utf8_first in [true, false] {
        let mut headers = HeaderMap::new();
        let invalid = HeaderValue::from_bytes(b"\xff").unwrap();
        if non_utf8_first {
            headers.append(header::COOKIE, invalid.clone());
        }
        headers.append(header::COOKIE, HeaderValue::from_static("session=abc"));
        if !non_utf8_first {
            headers.append(header::COOKIE, invalid);
        }
        for policy in [CookieHeaderPolicy::Strict, CookieHeaderPolicy::TargetOnly] {
            assert_eq!(read(&headers, policy), Err(CookieHeaderError::NonUtf8));
        }
    }
}

#[test]
fn cookie_header_errors_are_sanitized() {
    let secret = "secret-cookie-value";
    let headers = headers(&[&format!("session={secret}; session=other")]);
    let error = read(&headers, CookieHeaderPolicy::TargetOnly).unwrap_err();
    for text in [format!("{error:?}"), error.to_string()] {
        assert!(!text.contains(secret), "{text}");
    }
}
