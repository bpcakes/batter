//! Live-reader integration: startup decisions and publication during a request.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::StatusCode,
    routing::get,
};
use batter_axum::{
    GroupGate, GroupPolicy, GuardedRouter, HttpBoundary, InProcessClient, RequestPolicy,
    ResponseConstructionBudget,
};
use batter_core::lifecycle::ShutdownHandle;
use batter_flags_core::{
    CatalogBuilder, KillSwitch, ModeFlag, PrePublication, SafeState, SettingsPrefix, Source,
    StaleRule, SwitchState, Toggle, Version,
    low_level::{Base, BaseBuilder, Candidate, CandidateRow},
};
use tokio::{sync::Notify, time::Instant};

use super::AdmittedFlags;
use crate::{Flags, reader::Reader};

struct World {
    base: Base,
    released: KillSwitch,
    engaged: KillSwitch,
    stale: KillSwitch,
    floor: KillSwitch,
    flag: ModeFlag<Toggle>,
}

fn world() -> World {
    let (mut b, kind, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let released = b
        .kill_switch("released", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let engaged = b
        .kill_switch("engaged", SafeState::Engaged, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let stale = b
        .kill_switch("stale", SafeState::Released, StaleRule::EngageWhenStale)
        .declare()
        .unwrap();
    let floor = b
        .kill_switch("floor", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let flag = b.mode_flag("fast_path", Toggle::Off).declare().unwrap();
    let mut base = BaseBuilder::new(b.build(), kind).unwrap();
    base.switch(floor.key(), SwitchState::Engaged).unwrap();
    World {
        base: base.finish(),
        released,
        engaged,
        stale,
        floor,
        flag,
    }
}

async fn client(gate: GroupGate, routes: GuardedRouter) -> InProcessClient {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let policy = RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(5)).unwrap(),
    );
    HttpBoundary::new(GroupPolicy::gated(policy, gate))
        .assemble(routes)
        .await
        .unwrap()
        .in_process()
}

fn request() -> Request {
    Request::builder().uri("/work").body(Body::empty()).unwrap()
}

fn routes() -> GuardedRouter {
    GuardedRouter::new().route(
        "/work",
        get(|flags: AdmittedFlags| async move {
            assert!(matches!(flags.view().source(), Source::NotLoaded { .. }));
            "defaults"
        }),
    )
}

#[tokio::test]
async fn awaiting_publication_obeys_the_readers_startup_policy_and_every_switch() {
    for policy in [PrePublication::ServeDefaults, PrePublication::Unready] {
        let w = world();
        let (_writer, reader) =
            Reader::live(w.base, policy, Duration::from_secs(60), Instant::now());
        let flags = Flags::new(reader);
        let released_status = match policy {
            PrePublication::ServeDefaults => StatusCode::OK,
            PrePublication::Unready => StatusCode::SERVICE_UNAVAILABLE,
        };
        let cases = [
            (flags.http_view(), StatusCode::OK),
            (flags.http_gate(&w.released).unwrap(), released_status),
            (
                flags.http_gate(&w.engaged).unwrap(),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                flags.http_gate(&w.floor).unwrap(),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                flags.http_gate_any(&w.released, &[&w.stale]).unwrap(),
                released_status,
            ),
            (
                flags.http_gate_any(&w.released, &[&w.engaged]).unwrap(),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                flags.http_gate_any(&w.engaged, &[&w.released]).unwrap(),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ];
        for (gate, expected) in cases {
            assert_eq!(
                client(gate, routes())
                    .await
                    .request(request())
                    .await
                    .status(),
                expected
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn unpublished_defaults_apply_stale_rules_even_after_a_released_switch() {
    let w = world();
    let (_writer, reader) = Reader::live(
        w.base,
        PrePublication::ServeDefaults,
        Duration::from_secs(60),
        Instant::now(),
    );
    let flags = Flags::new(reader);
    let app = client(
        flags.http_gate_any(&w.released, &[&w.stale]).unwrap(),
        routes(),
    )
    .await;
    assert_eq!(app.request(request()).await.status(), StatusCode::OK);
    tokio::time::advance(Duration::from_secs(61)).await;
    assert_eq!(
        app.request(request()).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn publication_after_admission_changes_only_the_next_request() {
    let w = world();
    let scope = w.base.validation_scope();
    let candidate = |version, state, value| {
        let at = Version::new(version);
        Candidate::new(&scope, at)
            .row(CandidateRow::switch(&w.released, state, at))
            .row(CandidateRow::flag(&w.flag, value, at))
    };
    let first = candidate(1, SwitchState::Released, Toggle::On);
    let second = candidate(2, SwitchState::Engaged, Toggle::Off);
    let (mut writer, reader) = Reader::live(
        w.base,
        PrePublication::Unready,
        Duration::from_secs(60),
        Instant::now(),
    );
    writer.read_at(first, Instant::now());
    let flags = Flags::new(reader);
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let (handler_entered, handler_resume) = (entered.clone(), resume.clone());
    let app = client(
        flags.http_gate(&w.released).unwrap(),
        GuardedRouter::new().route(
            "/work",
            get(move |first: AdmittedFlags, second: AdmittedFlags| {
                let (entered, resume) = (handler_entered.clone(), handler_resume.clone());
                let (flag, switch) = (w.flag.clone(), w.released.clone());
                async move {
                    entered.notify_one();
                    resume.notified().await;
                    assert!(std::ptr::eq(first.view(), second.view()));
                    assert_eq!(first.view().evaluate(&flag).value(), Toggle::On);
                    assert_eq!(second.view().switch(&switch).value(), SwitchState::Released);
                    assert!(
                        matches!(second.view().source(), Source::ControlPlane { version, .. }
                    if version == Version::new(1))
                    );
                    "old publication"
                }
            }),
        ),
    )
    .await;
    let in_flight = app.request(request());
    let publication = async {
        entered.notified().await;
        writer.read_at(second, Instant::now());
        resume.notify_one();
    };
    let (response, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(in_flight, publication)
    })
    .await
    .expect("the admitted request must reach and leave its publication barrier");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "old publication"
    );
    assert_eq!(
        app.request(request()).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
