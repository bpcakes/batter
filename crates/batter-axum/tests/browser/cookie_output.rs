use axum::http::{HeaderMap, HeaderValue, header};
use batter_axum::browser::{
    BrowserCookie, BrowserOrigin, CookieLifetime, CookieMaxAge, CookieName, CookiePolicyError,
    CookieValue, CookieVisibility, SameSite, SetCookieError,
};
use cookie::Cookie;

fn secure_cookie(name: &str, same_site: SameSite, visibility: CookieVisibility) -> BrowserCookie {
    BrowserCookie::new(
        &BrowserOrigin::https("https://example.test").unwrap(),
        CookieName::new(name).unwrap(),
        same_site,
        visibility,
    )
    .unwrap()
}

fn parse(value: &HeaderValue) -> Cookie<'static> {
    Cookie::parse(value.to_str().unwrap().to_owned())
        .unwrap()
        .into_owned()
}

#[test]
fn secure_persistent_cookie_has_fixed_host_only_root_scope() {
    let cookie = secure_cookie("__Host-session", SameSite::Lax, CookieVisibility::HttpOnly);
    let mut headers = HeaderMap::new();
    cookie
        .append(
            &mut headers,
            CookieValue::new("opaque-token==").unwrap(),
            CookieLifetime::Persistent(CookieMaxAge::seconds(43_200).unwrap()),
        )
        .unwrap();
    let mut wire_parts = headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split("; ")
        .collect::<Vec<_>>();
    wire_parts.sort_unstable();
    assert_eq!(
        wire_parts,
        [
            "HttpOnly",
            "Max-Age=43200",
            "Path=/",
            "SameSite=Lax",
            "Secure",
            "__Host-session=opaque-token==",
        ]
    );
    let emitted = parse(&headers[header::SET_COOKIE]);
    assert_eq!(emitted.name(), "__Host-session");
    assert_eq!(emitted.value(), "opaque-token==");
    assert_eq!(emitted.path(), Some("/"));
    assert_eq!(emitted.domain(), None);
    assert_eq!(emitted.secure(), Some(true));
    assert_eq!(emitted.http_only(), Some(true));
    assert_eq!(emitted.same_site(), Some(cookie::SameSite::Lax));
    assert_eq!(emitted.max_age().unwrap().whole_seconds(), 43_200);
    assert_eq!(emitted.expires(), None);
}

#[test]
fn session_and_loopback_policies_omit_unselected_attributes() {
    let csrf = secure_cookie(
        "__Host-csrf",
        SameSite::Strict,
        CookieVisibility::ScriptReadable,
    );
    let mut headers = HeaderMap::new();
    csrf.append(
        &mut headers,
        CookieValue::new("opaque").unwrap(),
        CookieLifetime::Session,
    )
    .unwrap();
    let emitted = parse(&headers[header::SET_COOKIE]);
    assert_eq!(emitted.secure(), Some(true));
    assert_eq!(emitted.http_only(), None);
    assert_eq!(emitted.same_site(), Some(cookie::SameSite::Strict));
    assert_eq!(emitted.max_age(), None);
    assert_eq!(emitted.expires(), None);

    let local = BrowserCookie::new(
        &BrowserOrigin::loopback_http("http://localhost:3000").unwrap(),
        CookieName::new("local_session").unwrap(),
        SameSite::Lax,
        CookieVisibility::HttpOnly,
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    local
        .append(
            &mut headers,
            CookieValue::new("opaque").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    let emitted = parse(&headers[header::SET_COOKIE]);
    assert_eq!(emitted.path(), Some("/"));
    assert_eq!(emitted.domain(), None);
    assert_eq!(emitted.secure(), None);
    assert_eq!(emitted.http_only(), Some(true));

    let combined = secure_cookie(
        "__Host-Http-session",
        SameSite::Lax,
        CookieVisibility::HttpOnly,
    );
    let mut headers = HeaderMap::new();
    combined
        .append(
            &mut headers,
            CookieValue::new("opaque").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    let emitted = parse(&headers[header::SET_COOKIE]);
    assert_eq!(emitted.secure(), Some(true));
    assert_eq!(emitted.http_only(), Some(true));
}

#[test]
fn invalid_security_and_lifetime_combinations_fail_before_emission() {
    let secure = BrowserOrigin::https("https://example.test").unwrap();
    let local = BrowserOrigin::loopback_http("http://localhost").unwrap();
    for (origin, name, same_site, expected) in [
        (
            &secure,
            "session",
            SameSite::Lax,
            CookiePolicyError::SecureNameRequired,
        ),
        (
            &secure,
            "__host-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameRequired,
        ),
        (
            &secure,
            "__Secure-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameRequired,
        ),
        (
            &local,
            "__Host-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameNotAllowed,
        ),
        (
            &local,
            "__host-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameNotAllowed,
        ),
        (
            &local,
            "__Secure-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameNotAllowed,
        ),
        (
            &local,
            "__HTTP-session",
            SameSite::Lax,
            CookiePolicyError::SecureNameNotAllowed,
        ),
        (
            &local,
            "local_session",
            SameSite::None,
            CookiePolicyError::SameSiteNoneRequiresSecure,
        ),
    ] {
        assert_eq!(
            BrowserCookie::new(
                origin,
                CookieName::new(name).unwrap(),
                same_site,
                CookieVisibility::HttpOnly,
            ),
            Err(expected)
        );
    }
    assert_eq!(
        BrowserCookie::new(
            &secure,
            CookieName::new("__Host-Http-session").unwrap(),
            SameSite::Lax,
            CookieVisibility::ScriptReadable,
        ),
        Err(CookiePolicyError::HttpOnlyRequired)
    );
    assert_eq!(
        BrowserCookie::new(
            &secure,
            CookieName::new("__Host-http-session").unwrap(),
            SameSite::Lax,
            CookieVisibility::ScriptReadable,
        ),
        Err(CookiePolicyError::HttpOnlyRequired)
    );
    assert_eq!(
        CookieMaxAge::seconds(0),
        Err(CookiePolicyError::InvalidMaxAge)
    );
    assert_eq!(
        CookieMaxAge::seconds(i64::MAX as u64 + 1),
        Err(CookiePolicyError::InvalidMaxAge)
    );
    assert_eq!(
        CookieMaxAge::seconds(i64::MAX as u64).unwrap().get(),
        i64::MAX as u64
    );
}

#[test]
fn set_cookie_always_appends_and_preserves_field_order() {
    let session = secure_cookie("__Host-session", SameSite::Lax, CookieVisibility::HttpOnly);
    let csrf = secure_cookie(
        "__Host-csrf",
        SameSite::Strict,
        CookieVisibility::ScriptReadable,
    );
    let mut headers = HeaderMap::new();
    headers.append(header::SET_COOKIE, HeaderValue::from_static("unrelated=1"));
    session
        .append(
            &mut headers,
            CookieValue::new("session-value").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    csrf.append(
        &mut headers,
        CookieValue::new("csrf-value").unwrap(),
        CookieLifetime::Session,
    )
    .unwrap();
    let names: Vec<_> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| parse(value).name().to_owned())
        .collect();
    assert_eq!(names, ["unrelated", "__Host-session", "__Host-csrf"]);
}

#[test]
fn same_name_set_cookie_operations_fail_without_mutating_existing_fields() {
    let cookie = secure_cookie("__Host-session", SameSite::Lax, CookieVisibility::HttpOnly);

    let mut set_then_set = HeaderMap::new();
    cookie
        .append(
            &mut set_then_set,
            CookieValue::new("first").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    let original = set_then_set[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cookie.append(
            &mut set_then_set,
            CookieValue::new("second").unwrap(),
            CookieLifetime::Session,
        ),
        Err(SetCookieError::DuplicateName)
    );
    assert_eq!(set_then_set.get_all(header::SET_COOKIE).iter().count(), 1);
    assert_eq!(set_then_set[header::SET_COOKIE].to_str().unwrap(), original);

    let mut set_then_removal = HeaderMap::new();
    cookie
        .append(
            &mut set_then_removal,
            CookieValue::new("first").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    let original = set_then_removal[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cookie.append_removal(&mut set_then_removal),
        Err(SetCookieError::DuplicateName)
    );
    assert_eq!(
        set_then_removal.get_all(header::SET_COOKIE).iter().count(),
        1
    );
    assert_eq!(
        set_then_removal[header::SET_COOKIE].to_str().unwrap(),
        original
    );

    let mut removal_then_set = HeaderMap::new();
    cookie.append_removal(&mut removal_then_set).unwrap();
    let original = removal_then_set[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cookie.append(
            &mut removal_then_set,
            CookieValue::new("new").unwrap(),
            CookieLifetime::Session,
        ),
        Err(SetCookieError::DuplicateName)
    );
    assert_eq!(
        removal_then_set.get_all(header::SET_COOKIE).iter().count(),
        1
    );
    assert_eq!(
        removal_then_set[header::SET_COOKIE].to_str().unwrap(),
        original
    );
    assert_eq!(
        SetCookieError::DuplicateName.to_string(),
        "response already contains the browser cookie name"
    );

    let mut external = HeaderMap::new();
    external.append(
        header::SET_COOKIE,
        HeaderValue::from_bytes(b" \t__Host-session \t=external; Path=/").unwrap(),
    );
    let original = external[header::SET_COOKIE].to_str().unwrap().to_owned();
    assert_eq!(
        cookie.append(
            &mut external,
            CookieValue::new("new").unwrap(),
            CookieLifetime::Session,
        ),
        Err(SetCookieError::DuplicateName)
    );
    assert_eq!(external.get_all(header::SET_COOKIE).iter().count(), 1);
    assert_eq!(external[header::SET_COOKIE].to_str().unwrap(), original);
}

#[test]
fn same_name_response_check_is_case_sensitive_and_atomic() {
    let cookie = secure_cookie("__Host-session", SameSite::Lax, CookieVisibility::HttpOnly);
    let case_distinct = secure_cookie("__Host-Session", SameSite::Lax, CookieVisibility::HttpOnly);
    let mut distinct = HeaderMap::new();
    cookie
        .append(
            &mut distinct,
            CookieValue::new("lower").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    case_distinct
        .append(
            &mut distinct,
            CookieValue::new("upper").unwrap(),
            CookieLifetime::Session,
        )
        .unwrap();
    let original: Vec<_> = distinct
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.as_bytes().to_owned())
        .collect();
    assert_eq!(original.len(), 2);
    assert_eq!(
        cookie.append(
            &mut distinct,
            CookieValue::new("duplicate").unwrap(),
            CookieLifetime::Session,
        ),
        Err(SetCookieError::DuplicateName)
    );
    assert_eq!(
        distinct
            .get_all(header::SET_COOKIE)
            .iter()
            .map(HeaderValue::as_bytes)
            .collect::<Vec<_>>(),
        original.iter().map(Vec::as_slice).collect::<Vec<_>>()
    );
}

#[test]
fn secure_same_site_none_and_maximum_age_serialize_without_weakening_attributes() {
    let cookie = secure_cookie(
        "__Host-cross-site",
        SameSite::None,
        CookieVisibility::HttpOnly,
    );
    let mut headers = HeaderMap::new();
    cookie
        .append(
            &mut headers,
            CookieValue::new("opaque").unwrap(),
            CookieLifetime::Persistent(CookieMaxAge::seconds(i64::MAX as u64).unwrap()),
        )
        .unwrap();
    let emitted = parse(&headers[header::SET_COOKIE]);
    assert_eq!(emitted.same_site(), Some(cookie::SameSite::None));
    assert_eq!(emitted.secure(), Some(true));
    assert_eq!(emitted.http_only(), Some(true));
    assert_eq!(emitted.max_age().unwrap().whole_seconds(), i64::MAX);

    let mut removal = HeaderMap::new();
    cookie.append_removal(&mut removal).unwrap();
    let emitted = parse(&removal[header::SET_COOKIE]);
    assert_eq!(emitted.same_site(), Some(cookie::SameSite::None));
    assert_eq!(emitted.secure(), Some(true));
}

#[test]
fn removal_matches_each_policy_and_retains_existing_fields() {
    let secure = secure_cookie("__Host-session", SameSite::Lax, CookieVisibility::HttpOnly);
    let local = BrowserCookie::new(
        &BrowserOrigin::loopback_http("http://localhost").unwrap(),
        CookieName::new("local_session").unwrap(),
        SameSite::Strict,
        CookieVisibility::ScriptReadable,
    )
    .unwrap();
    for cookie in [secure, local] {
        let mut headers = HeaderMap::new();
        headers.append(header::SET_COOKIE, HeaderValue::from_static("unrelated=1"));
        cookie.append_removal(&mut headers).unwrap();
        let values: Vec<_> = headers.get_all(header::SET_COOKIE).iter().collect();
        assert_eq!(values.len(), 2);
        let emitted = parse(values[1]);
        assert_eq!(emitted.name(), cookie.name().as_str());
        assert_eq!(emitted.value(), "");
        assert_eq!(emitted.path(), Some("/"));
        assert_eq!(emitted.domain(), None);
        assert_eq!(
            emitted.same_site(),
            Some(match cookie.same_site() {
                SameSite::Strict => cookie::SameSite::Strict,
                SameSite::Lax => cookie::SameSite::Lax,
                SameSite::None => cookie::SameSite::None,
            })
        );
        assert_eq!(emitted.secure(), cookie.is_secure().then_some(true));
        assert_eq!(
            emitted.http_only(),
            (cookie.visibility() == CookieVisibility::HttpOnly).then_some(true)
        );
        assert_eq!(emitted.max_age().unwrap().whole_seconds(), 0);
        assert!(emitted.expires_datetime().unwrap() < cookie::time::OffsetDateTime::now_utc());
    }
}

#[test]
fn invalid_values_cannot_inject_cookie_attributes_or_fields() {
    for value in [
        "opaque; Domain=evil.test",
        "opaque\r\nSet-Cookie: injected=1",
    ] {
        assert!(CookieValue::new(value).is_err());
    }
}
