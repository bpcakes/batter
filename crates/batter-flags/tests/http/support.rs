//! Small HTTP fixture; requests always cross the sealed boundary.
#![allow(dead_code, reason = "shared support; each HTTP test uses part of it")]

use std::time::Duration;

use axum::{body::Body, extract::Request};
use batter_axum::{
    GroupGate, GroupPolicy, GuardedRouter, HttpBoundary, InProcessClient, RequestPolicy,
    ResponseConstructionBudget, RouteGroup,
};
use batter_core::lifecycle::ShutdownHandle;
use batter_flags::{
    core::{
        Catalog, CatalogBuilder, KillSwitch, ModeFlag, ProcessKind, SafeState, SettingsPrefix,
        StaleRule, Toggle,
    },
    test::TestFlags,
};

pub struct World {
    pub serving: ProcessKind,
    pub halt: KillSwitch,
    pub other: KillSwitch,
    pub worker_only: KillSwitch,
    pub flag: ModeFlag<Toggle>,
}

pub fn world() -> (Catalog, World) {
    let (mut b, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let worker = b.process_kind("worker").unwrap();
    let halt = b
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let other = b
        .kill_switch(
            "other_halt",
            SafeState::Released,
            StaleRule::EngageWhenStale,
        )
        .declare()
        .unwrap();
    let worker_only = b
        .kill_switch("worker_halt", SafeState::Released, StaleRule::HoldLastKnown)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    let flag = b.mode_flag("fast_path", Toggle::Off).declare().unwrap();
    (
        b.build(),
        World {
            serving,
            halt,
            other,
            worker_only,
            flag,
        },
    )
}

pub fn fixture(catalog: Catalog, w: &World, engaged: &[&KillSwitch]) -> TestFlags {
    let mut builder = TestFlags::builder(catalog, w.serving.clone())
        .unwrap()
        .set(&w.flag, Toggle::On);
    for switch in engaged {
        builder = builder.engage(switch);
    }
    builder.build().unwrap()
}

pub fn policy() -> RequestPolicy {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    )
}

pub async fn client(gate: GroupGate, routes: GuardedRouter) -> InProcessClient {
    let policy = policy();
    HttpBoundary::new(policy.clone())
        .with_group(RouteGroup::new(
            "work",
            GroupPolicy::gated(policy, gate),
            routes,
        ))
        .unwrap()
        .assemble(GuardedRouter::new())
        .await
        .unwrap()
        .in_process()
}

pub fn request() -> Request {
    Request::builder().uri("/work").body(Body::empty()).unwrap()
}
