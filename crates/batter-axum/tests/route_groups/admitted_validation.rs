//! Admitted routers that assembly must reject before any request is routed.

use super::support::{SECOND, request_policy};
use axum::{
    Router,
    extract::Request,
    middleware::{self, Next},
    routing::{MethodRouter, post},
};
use batter_axum::{
    AssembledHttp, BoundaryAssemblyError, GuardedRouter, HttpBoundary, ProbePath, RouteGroup,
    RouteInventory, RouteInventoryError,
};
use batter_core::lifecycle::ShutdownHandle;
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Application code built with a plain Axum router: handlers, method
/// fallbacks, a nested service, route and router middleware and a fallback
/// all share one call counter.
struct Application(Arc<AtomicUsize>);

impl Application {
    fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }

    fn counted(
        &self,
    ) -> impl Fn() -> std::future::Ready<&'static str> + Clone + Send + Sync + 'static {
        let calls = self.0.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready("handled")
        }
    }

    /// A POST-only route whose method fallback is also application code, so
    /// inspecting any other method would reach it.
    fn route(&self) -> MethodRouter {
        post(self.counted()).fallback(self.counted())
    }

    fn router(&self, paths: &[&str]) -> Router {
        let router = paths.iter().fold(Router::new(), |router, path| {
            router.route(path, self.route())
        });
        let (route_calls, layer_calls) = (self.0.clone(), self.0.clone());
        let router = if router.has_routes() {
            router.route_layer(middleware::from_fn(move |request: Request, next: Next| {
                route_calls.fetch_add(1, Ordering::SeqCst);
                async move { next.run(request).await }
            }))
        } else {
            router
        };
        router
            .layer(middleware::from_fn(move |request: Request, next: Next| {
                layer_calls.fetch_add(1, Ordering::SeqCst);
                async move { next.run(request).await }
            }))
            .fallback(self.counted())
    }

    fn nested_service(&self, path: &str) -> Router {
        let calls = self.0.clone();
        let service = tower::service_fn(move |_request: Request| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>("nested") }
        });
        self.router(&["/items"]).nest_service(path, service)
    }

    fn admitted(&self, router: Router, declared: &[&str]) -> GuardedRouter {
        let inventory = RouteInventory::new(declared.iter().copied()).unwrap();
        GuardedRouter::from_router(router, inventory)
    }

    fn native(&self, paths: &[&str]) -> GuardedRouter {
        paths.iter().fold(GuardedRouter::new(), |router, path| {
            router.route(path, self.route())
        })
    }

    fn assert_untouched(&self) {
        assert_eq!(self.0.load(Ordering::SeqCst), 0);
    }
}

async fn assemble(
    default: GuardedRouter,
    items: Option<GuardedRouter>,
    probes: &[&'static str],
) -> Result<AssembledHttp, BoundaryAssemblyError> {
    let handle = ShutdownHandle::new_unapproved();
    let mut boundary = HttpBoundary::new(request_policy(&handle, SECOND));
    for probe in probes {
        boundary = boundary
            .with_liveness(ProbePath::new(probe).unwrap())
            .unwrap();
    }
    if let Some(items) = items {
        let group = RouteGroup::new("items", request_policy(&handle, SECOND), items);
        boundary = boundary.with_group(group).unwrap();
    }
    boundary.assemble(default).await
}

#[tokio::test]
async fn admitted_routes_matching_a_probe_are_rejected_without_polling() {
    let app = Application::new();
    let cases = [
        // A declared route at the probe path.
        (
            app.router(&["/items", "/live"]),
            &["/items", "/live"][..],
            "/live",
        ),
        // Undeclared routes are inspected too, whatever their method.
        (app.router(&["/items", "/live"]), &["/items"][..], "/live"),
        (
            app.router(&["/items", "/{*path}"]),
            &["/items"][..],
            "/internal/live",
        ),
        // An opaque nested service covers every path below it.
        (
            app.nested_service("/internal"),
            &["/items"][..],
            "/internal/live",
        ),
    ];
    for (router, declared, probe) in cases {
        for named in [false, true] {
            let admitted = app.admitted(router.clone(), declared);
            let (default, items) = if named {
                (app.native(&["/work"]), Some(admitted))
            } else {
                (admitted, None)
            };
            let result = assemble(default, items, &[probe]).await;
            assert_eq!(
                result.err(),
                Some(BoundaryAssemblyError::GuardedProbePath),
                "{probe} {declared:?}"
            );
        }
    }
    app.assert_untouched();
}

#[tokio::test]
async fn inventory_mismatches_are_rejected_without_polling() {
    let app = Application::new();
    let cases = [
        // A declared pattern the router does not register.
        (app.router(&["/items"]), &["/items", "/items/{id}"][..]),
        // Parameter names are part of the registered pattern.
        (app.router(&["/items/{id}"]), &["/items/{item}"][..]),
        // A declared literal is served by a wildcard, not by its own route.
        (app.router(&["/items/{*path}"]), &["/items/new"][..]),
        // Opaque nested services expose no route pattern.
        (app.nested_service("/internal"), &["/internal/items"][..]),
        // A path that no request URI can carry cannot be served.
        (app.router(&["/a b"]), &["/a b"][..]),
        (app.router(&[]), &["/items"][..]),
    ];
    for (router, declared) in cases {
        let admitted = app.admitted(router.clone(), declared);
        let result = assemble(admitted, None, &[]).await;
        let expected = BoundaryAssemblyError::RouteInventoryMismatch { group: "default" };
        assert_eq!(result.err(), Some(expected), "{declared:?}");

        let admitted = app.admitted(router, declared);
        let result = assemble(app.native(&["/work"]), Some(admitted), &[]).await;
        let expected = BoundaryAssemblyError::RouteInventoryMismatch { group: "items" };
        assert_eq!(result.err(), Some(expected), "{declared:?}");
    }
    app.assert_untouched();
    assert_eq!(
        BoundaryAssemblyError::RouteInventoryMismatch { group: "items" }.to_string(),
        "route inventory in group items does not match its router"
    );
}

#[tokio::test]
async fn sibling_routes_do_not_take_inventory_witnesses() {
    let app = Application::new();
    for (routes, declared) in [
        // Literals that equal conventional capture and wildcard fillers.
        (&["/items/{id}", "/items/x"][..], &["/items/{id}"][..]),
        (
            &["/files/{*path}", "/files/x", "/files/x/x"],
            &["/files/{*path}"],
        ),
        // A capture with a literal prefix beside one without.
        (
            &["/v{version}/items", "/{section}/items"],
            &["/{section}/items"],
        ),
        (&["/", "/{*path}"], &["/", "/{*path}"]),
    ] {
        let admitted = app.admitted(app.router(routes), declared);
        let result = assemble(admitted, None, &[]).await;
        assert!(result.is_ok(), "{declared:?}: {:?}", result.err());
    }
    app.assert_untouched();
}

#[tokio::test]
async fn admitted_routes_overlapping_other_routes_are_rejected_without_polling() {
    let app = Application::new();
    // Against another group, a capture would shadow the other group's literal.
    let admitted = app.admitted(app.router(&["/files/{id}"]), &["/files/{id}"]);
    let result = assemble(app.native(&["/files/new"]), Some(admitted), &[]).await;
    let expected = BoundaryAssemblyError::OverlappingGroupPaths {
        first: "default",
        second: "items",
    };
    assert_eq!(result.err(), Some(expected));

    // Within one group, against native routes or another admitted router.
    let same_group = [
        app.native(&["/files/new"])
            .merge(app.admitted(app.router(&["/files/{id}"]), &["/files/{id}"])),
        app.admitted(app.router(&["/files/{id}"]), &["/files/{id}"])
            .merge(app.admitted(app.router(&["/files/{*path}"]), &["/files/{*path}"])),
        GuardedRouter::new()
            .nest("/files", app.admitted(app.router(&["/{id}"]), &["/{id}"]))
            .merge(app.native(&["/files/new"])),
    ];
    for routes in same_group {
        let result = assemble(app.native(&["/work"]), Some(routes), &[]).await;
        let expected = BoundaryAssemblyError::OverlappingRouteInventory { group: "items" };
        assert_eq!(result.err(), Some(expected));
    }
    app.assert_untouched();
    assert_eq!(
        BoundaryAssemblyError::OverlappingRouteInventory { group: "items" }.to_string(),
        "route inventory in group items can match the same path as other routes of that group"
    );
}

#[tokio::test]
async fn undeclared_routes_take_no_part_in_overlap() {
    let app = Application::new();
    // The undeclared `/files/{id}` never serves, so it cannot overlap.
    let admitted = app.admitted(app.router(&["/items", "/files/{id}"]), &["/items"]);
    let default = app
        .native(&["/files/new"])
        .fallback(|| async { "fallback" });
    let assembled = assemble(default, Some(admitted), &["/live"]).await;
    assert!(assembled.is_ok(), "{:?}", assembled.err());
    app.assert_untouched();
}

#[tokio::test]
async fn a_capture_name_may_start_with_a_slash() {
    let app = Application::new();
    // matchit reads a parameter name's first byte before looking for `/`.
    for named in [false, true] {
        let admitted = app.admitted(app.router(&["/items/{/id}"]), &["/items/{/id}"]);
        let (default, items) = if named {
            (app.native(&["/work"]), Some(admitted))
        } else {
            (admitted, None)
        };
        let assembled = assemble(default, items, &["/live"]).await;
        assert!(assembled.is_ok(), "{:?}", assembled.err());
    }
    // A native route with such a name no longer fails the overlap analysis.
    let items = app.native(&["/other"]);
    let assembled = assemble(app.native(&["/items/{/id}"]), Some(items), &[]).await;
    assert!(assembled.is_ok(), "{:?}", assembled.err());
    app.assert_untouched();
}

#[tokio::test]
async fn a_named_group_may_consist_of_admitted_routers() {
    let app = Application::new();
    let admitted = app.admitted(app.router(&["/items"]), &["/items"]);
    assert!(
        assemble(app.native(&["/work"]), Some(admitted), &[])
            .await
            .is_ok()
    );
    app.assert_untouched();
}

#[test]
fn inventories_need_valid_absolute_patterns() {
    assert_eq!(
        RouteInventory::new(Vec::<&str>::new()),
        Err(RouteInventoryError::Empty)
    );
    // Parameter names that Axum's router refuses are rejected here too.
    let unnamed = ["/{}", "/{*}", "/{a*b}", "/{**}", "/{}}a}"];
    for (index, invalid) in ["", "items", "/{id", "/id}", "/{*path}/more", "/{id}x"]
        .into_iter()
        .chain(unnamed)
        .enumerate()
    {
        let patterns = ["/valid", invalid];
        assert_eq!(
            RouteInventory::new(patterns),
            Err(RouteInventoryError::InvalidPattern { index: 1 }),
            "{index}: {invalid}"
        );
    }
    assert_eq!(
        RouteInventory::new(["/items", "/items/{id}", "/items"]),
        RouteInventory::new([String::from("/items"), String::from("/items/{id}")])
    );
    assert!(RouteInventory::new(["/", "/{{literal}}", "/v{version}", "/a/{*rest}"]).is_ok());
    assert_eq!(
        RouteInventoryError::Empty.to_string(),
        "route inventory declares no route"
    );
    assert_eq!(
        RouteInventoryError::InvalidPattern { index: 3 }.to_string(),
        "route inventory pattern 3 is invalid"
    );
}
