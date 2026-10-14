use super::{Index, Segment, SharedPath, parse, shared_path};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::any,
};
use tower::ServiceExt;

fn literal(bytes: &str) -> Segment {
    Segment::Literal(bytes.as_bytes().to_vec())
}

fn capture(prefix: &str) -> Segment {
    Segment::Capture(prefix.as_bytes().to_vec())
}

fn wildcard(prefix: &str) -> Segment {
    Segment::Wildcard(prefix.as_bytes().to_vec())
}

#[test]
fn parses_literals_escapes_captures_and_wildcards() {
    assert_eq!(parse("/"), Some(vec![literal("")]));
    assert_eq!(
        parse("/a//b/"),
        Some(vec![literal("a"), literal(""), literal("b"), literal("")])
    );
    assert_eq!(
        parse("/{{id}}/v{version}/{*rest}"),
        Some(vec![literal("{id}"), capture("v"), wildcard("")])
    );
    assert_eq!(parse("/files-{*path}"), Some(vec![wildcard("files-")]));
    // Escaped braces may also appear inside a parameter name or before it.
    assert_eq!(parse("/{a}}b}"), Some(vec![capture("")]));
    assert_eq!(parse("/{{{id}"), Some(vec![capture("{")]));
    // A name's first byte may be `/` without splitting the segment.
    assert_eq!(
        parse("/items/{/id}"),
        Some(vec![literal("items"), capture("")])
    );
    assert_eq!(
        parse("/{/id}/{*rest}"),
        Some(vec![capture(""), wildcard("")])
    );
    for invalid in [
        "",
        "relative",
        "/{id}.json",
        "/{*rest}/tail",
        "/{id",
        "/}",
        "/{{a}",
        "/{a}}",
        "/{}",
        "/{*}",
        "/{a*b}",
        "/{**}",
        "/{*a*}",
        "/{}}a}",
        "/{a/b}",
        "/{/a/b}",
        "/{*/a}",
        "/{/*}",
        "/{/}x",
    ] {
        assert_eq!(parse(invalid), None, "{invalid}");
    }
    assert_eq!(shared_path("/{id}.json", "/a"), SharedPath::Unanalyzed);
}

/// Whether the pinned Axum router registers `pattern` on its own.
fn axum_registers(pattern: &str) -> bool {
    std::panic::catch_unwind(|| {
        let _ = Router::<()>::new()
            .without_v07_checks()
            .route(pattern, any(|| async {}));
    })
    .is_ok()
}

#[test]
fn parse_accepts_exactly_the_patterns_axum_registers() {
    let captures =
        |count: usize| -> String { (0..count).map(|index| format!("/{{p{index}}}")).collect() };
    let (most, too_many) = (captures(25), captures(26));
    for pattern in [
        "/",
        "/a",
        "/{a}",
        "/v{a}",
        "/{*a}",
        "/v{*a}",
        "/{a}/{*b}",
        "/{{a}}",
        "/{a}}b}",
        "/{a{b}",
        "/{{{a}",
        "/{*a}}b}",
        "/{-}",
        "/{*-}",
        "/items/{/id}",
        "/{/}",
        "/{/id}/{*rest}",
        "/{a/b}",
        "/{/a/b}",
        "/{*/a}",
        "/{/*}",
        "/{/}x",
        "/{}",
        "/{*}",
        "/{a*b}",
        "/{**}",
        "/{*a*}",
        "/{}}a}",
        "/{a",
        "/a}",
        "/{a}b",
        "/{a}{b}",
        "/{*a}/b",
        most.as_str(),
        too_many.as_str(),
    ] {
        assert_eq!(
            parse(pattern).is_some(),
            axum_registers(pattern),
            "{pattern}"
        );
    }
}

#[test]
fn common_application_patterns_share_only_real_paths() {
    for (first, second, expected) in [
        ("/files/{id}", "/files/new", Some("/files/new")),
        ("/files/{id}", "/files/{id}/content", None),
        ("/items", "/items", Some("/items")),
        ("/api/{*rest}", "/api", None),
        ("/api/{*rest}", "/api/", None),
        ("/api/{*rest}", "/api/v1/items", Some("/api/v1/items")),
        ("/v{version}/items", "/v1/items", Some("/v1/items")),
        ("/v{version}/items", "/api/items", None),
        (
            "/{tenant}/reports",
            "/uploads/{id}",
            Some("/uploads/reports"),
        ),
        ("/{tenant}/reports", "/uploads/{id}/reports", None),
        ("/{tenant}/{id}", "/uploads/{*path}", Some("/uploads/x")),
        ("/users/{id}", "/users/", None),
        (
            "/users/{id}/avatar",
            "/users//avatar",
            Some("/users//avatar"),
        ),
    ] {
        let expected = expected.map_or(SharedPath::Disjoint, |path| {
            SharedPath::Witness(path.to_owned())
        });
        assert_eq!(shared_path(first, second), expected, "{first} {second}");
        let reversed = shared_path(second, first);
        assert_eq!(
            matches!(reversed, SharedPath::Disjoint),
            matches!(expected, SharedPath::Disjoint),
            "{second} {first}"
        );
    }
}

const PATTERNS: &[&str] = &[
    "/",
    "/a",
    "/b",
    "/ab",
    "/a/",
    "//",
    "/a//b",
    "/a/b",
    "/{{x}}",
    "/{x}",
    "/a{x}",
    "/ab{x}",
    "/{x}/",
    "/{x}/b",
    "/a/{x}",
    "/{x}/{y}",
    "/a{x}/b",
    "/{*r}",
    "/a{*r}",
    "/ab{*r}",
    "/a/{*r}",
    "/{x}/{*r}",
    "/a/b/{*r}",
    "/{/x}",
    "/a/{/x}",
];

/// Every constructed witness segment is drawn from these literals and prefixes.
const SEGMENTS: &[&str] = &["", "a", "b", "ab", "x", "ax", "abx", "{x}"];

fn candidate_paths() -> Vec<String> {
    let mut paths: Vec<String> = SEGMENTS.iter().map(|s| format!("/{s}")).collect();
    let mut previous = paths.clone();
    for _ in 1..3 {
        previous = previous
            .iter()
            .flat_map(|path| SEGMENTS.iter().map(move |s| format!("{path}/{s}")))
            .collect();
        paths.extend(previous.iter().cloned());
    }
    paths
}

async fn routes(pattern: &str, path: &str) -> bool {
    let router: Router = Router::new().route(pattern, any(|| async { StatusCode::NO_CONTENT }));
    let Ok(request) = Request::builder().uri(path).body(Body::empty()) else {
        return false;
    };
    router.oneshot(request).await.unwrap().status() == StatusCode::NO_CONTENT
}

/// Compare the analysis with native Axum routing over every short path built
/// from the segments a witness can contain.
#[tokio::test]
async fn shared_paths_agree_with_native_axum_routing() {
    let paths = candidate_paths();
    let mut matched = Vec::new();
    for pattern in PATTERNS {
        let mut routed = Vec::new();
        for path in &paths {
            routed.push(routes(pattern, path).await);
        }
        matched.push(routed);
    }
    for (first_index, first) in PATTERNS.iter().enumerate() {
        for (second_index, second) in PATTERNS.iter().enumerate() {
            let routed_overlap = (0..paths.len())
                .any(|path| matched[first_index][path] && matched[second_index][path]);
            match shared_path(first, second) {
                SharedPath::Witness(path) => {
                    assert!(routes(first, &path).await, "{first} rejects {path}");
                    assert!(routes(second, &path).await, "{second} rejects {path}");
                }
                SharedPath::Disjoint => {
                    assert!(!routed_overlap, "{first} and {second} share a routed path");
                }
                SharedPath::Unanalyzed => panic!("{first} and {second} were not analyzed"),
            }
        }
    }
}

/// Every router the index names for `path`, in the order dispatch offers them.
fn candidates(index: &Index, path: &str) -> Vec<usize> {
    let mut candidates = Vec::new();
    while let Some(router) = index.candidate(path, candidates.last().map_or(0, |last| last + 1)) {
        candidates.push(router);
    }
    candidates
}

/// With every pattern as its own router, the index names exactly the patterns
/// that native Axum routing of each pattern alone matches, for every short
/// path built from the segments a witness can contain.
#[tokio::test]
async fn index_names_exactly_the_patterns_native_axum_routing_matches() {
    let index = Index::new(PATTERNS.iter().map(|pattern| [pattern]));
    for path in candidate_paths() {
        let mut routed = Vec::new();
        for (router, pattern) in PATTERNS.iter().enumerate() {
            if routes(pattern, &path).await {
                routed.push(router);
            }
        }
        assert_eq!(candidates(&index, &path), routed, "{path}");
    }
}

#[test]
fn index_names_each_router_once_and_unanalyzed_routers_always() {
    let index = Index::new([
        vec!["/items/{id}", "/items/{*rest}", "/items/{id}/parts"],
        vec!["/{id}.json"],
        vec!["/reports"],
    ]);
    assert_eq!(candidates(&index, "/items/7"), [0, 1]);
    assert_eq!(candidates(&index, "/items/7/parts"), [0, 1]);
    assert_eq!(candidates(&index, "/reports"), [1, 2]);
    assert_eq!(candidates(&index, "/missing"), [1]);
    assert_eq!(candidates(&index, "*"), [1]);
    let analyzed = Index::new([["/items/{id}"], ["/reports"]]);
    assert!(analyzed.candidate("/missing", 0).is_none());
    assert!(analyzed.candidate("*", 0).is_none());
}
