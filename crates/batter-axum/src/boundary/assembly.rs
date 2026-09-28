//! Validation and composition of the route groups one boundary assembles.

use super::{
    BoundaryAssemblyError, GroupPolicy, GuardedRouter, ProbePath,
    declared::{Admitted, DeclaredRouter, Dispatch, Inspection},
    inventory,
};
use crate::operational_http;
use axum::{Router, middleware};

/// One route group's native routes and admitted routers during assembly.
pub(super) struct Assembling {
    name: &'static str,
    policy: GroupPolicy,
    router: Router,
    patterns: Vec<String>,
    declared: Vec<Declared>,
}

/// A router admitted with its inventory, and its inspection copy.
struct Declared {
    router: Router,
    patterns: Vec<String>,
    inspection: Inspection,
}

impl Assembling {
    pub(super) fn new(name: &'static str, policy: GroupPolicy, routes: GuardedRouter) -> Self {
        let GuardedRouter {
            router,
            route_patterns,
            declares_fallback: _,
            declared,
        } = routes;
        let declared = declared
            .into_iter()
            .map(|DeclaredRouter { router, patterns }| Declared {
                inspection: Inspection::new(&router),
                router,
                patterns,
            })
            .collect();
        Self {
            name,
            policy,
            router,
            patterns: route_patterns,
            declared,
        }
    }

    /// Every pattern that can route a request into this group.
    fn all_patterns(&self) -> Vec<String> {
        self.declared
            .iter()
            .flat_map(|declared| &declared.patterns)
            .chain(&self.patterns)
            .cloned()
            .collect()
    }

    /// Install the group's policy around its native routes, and around each
    /// admitted router together with its own observer.
    fn install(self, admitted: &mut Vec<Admitted>) -> Router {
        let Self {
            policy,
            router,
            declared,
            ..
        } = self;
        for Declared {
            router,
            patterns,
            inspection,
        } in declared
        {
            let served = policy
                .clone()
                .apply(router)
                .layer(middleware::from_fn(operational_http));
            admitted.push(Admitted::new(inspection, patterns, served));
        }
        policy.apply(router)
    }
}

/// Validate the groups, the default one first, then compose them.
///
/// Named groups are merged into the probes and the default group last, with
/// correlation and the single observer outermost on every route. Admitted
/// routers are served in front of that router, each inside its group's policy
/// and its own observer, for exactly the requests they route to a declared
/// pattern; they are never merged into it.
pub(super) async fn assemble(
    probes: Router,
    probe_paths: &[ProbePath],
    groups: Vec<Assembling>,
) -> Result<Router, BoundaryAssemblyError> {
    validate(&groups, probe_paths).await?;
    let mut groups = groups.into_iter();
    let default = groups.next().expect("the default group is assembled first");
    let mut admitted = Vec::new();
    let mut router = probes;
    for group in groups {
        router = router.merge(group.install(&mut admitted));
    }
    // With two default fallbacks Axum retains the second router's; a second
    // custom fallback also supersedes a first default. Named groups declare
    // no fallback, so merging the default group last retains its layered one.
    let router = router
        .merge(default.install(&mut admitted))
        .layer(middleware::from_fn(operational_http));
    Ok(if admitted.is_empty() {
        router
    } else {
        Router::new().fallback_service(Dispatch::new(admitted, router))
    })
}

/// Reject inventory mismatches, then probe collisions in any group, then
/// overlapping group paths, then admitted routes overlapping their own group.
async fn validate(
    groups: &[Assembling],
    probes: &[ProbePath],
) -> Result<(), BoundaryAssemblyError> {
    for group in groups {
        for declared in &group.declared {
            if !declared
                .inspection
                .serves_inventory(&declared.patterns)
                .await
            {
                return Err(BoundaryAssemblyError::RouteInventoryMismatch { group: group.name });
            }
        }
    }
    for group in groups {
        if inventory::claims_probe(&group.patterns, probes).await {
            return Err(BoundaryAssemblyError::GuardedProbePath);
        }
        for declared in &group.declared {
            if declared.inspection.claims_probe(probes).await {
                return Err(BoundaryAssemblyError::GuardedProbePath);
            }
        }
    }
    let patterns: Vec<Vec<String>> = groups.iter().map(Assembling::all_patterns).collect();
    let inventories: Vec<(&'static str, &[String])> = groups
        .iter()
        .zip(&patterns)
        .map(|(group, patterns)| (group.name, patterns.as_slice()))
        .collect();
    if let Some((first, second)) = inventory::overlapping_groups(&inventories).await {
        return Err(BoundaryAssemblyError::OverlappingGroupPaths { first, second });
    }
    for group in groups {
        if declared_overlap(group).await {
            return Err(BoundaryAssemblyError::OverlappingRouteInventory { group: group.name });
        }
    }
    Ok(())
}

/// Whether an admitted router's declared routes share a request path with the
/// group's native routes or with another admitted router of the group.
///
/// Admitted routers are consulted before native routing, so such a path would
/// otherwise be served by whichever router is consulted first rather than by
/// Axum's route priority.
async fn declared_overlap(group: &Assembling) -> bool {
    for (index, declared) in group.declared.iter().enumerate() {
        if inventory::share_path(&declared.patterns, &group.patterns).await {
            return true;
        }
        for other in &group.declared[index + 1..] {
            if inventory::share_path(&declared.patterns, &other.patterns).await {
                return true;
            }
        }
    }
    false
}
