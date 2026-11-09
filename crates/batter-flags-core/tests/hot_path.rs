//! A criterion-free micro-benchmark of the evaluation hot path.
//!
//! Ignored by default, because a timing is not a pass or a fail:
//!
//! ```text
//! cargo test --release -p batter-flags-core --test hot_path -- --ignored --nocapture
//! ```
//!
//! It prints nanoseconds per call for each surface under three dispatchers: none
//! current when the view was captured, one that filters the event out, and one
//! that receives it. The numbers are unverified: one local run on one machine
//! says nothing about another, and this is the case contributed to the
//! `batter-xgy4` benchmark method, not a measurement it accepts.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use batter_flags_core::{
    Freshness, SnapshotView, SwitchState, Toggle,
    low_level::{Base, gate_switch},
};
use tracing_subscriber::{
    Layer, Registry, filter::LevelFilter, layer::SubscriberExt, registry::LookupSpan,
};
use world::{Handles, Literal, base, flag_row, snapshot, switch_row, world};

const ITERATIONS: u32 = 500_000;

/// Counts the events it receives and does nothing else.
struct Count(Arc<AtomicU64>);

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Count {
    fn on_event(&self, _: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn dispatch(maximum: LevelFilter) -> (tracing::Dispatch, Arc<AtomicU64>) {
    let received = Arc::new(AtomicU64::new(0));
    let layer = Count(Arc::clone(&received)).with_filter(maximum);
    (
        test_dispatch::new(Registry::default().with(layer)),
        received,
    )
}

fn views(base: &Base, h: &Handles) -> SnapshotView {
    let published = snapshot(
        base,
        7,
        [
            flag_row(&h.fast_path, Toggle::On, 6),
            flag_row(&h.new_checkout, Toggle::On, 6),
            switch_row(&h.checkout_halt, SwitchState::Released, 6),
        ],
    );
    base.control_plane_view(&published, Freshness::Fresh)
        .unwrap()
}

fn time(label: &str, mut call: impl FnMut()) {
    for _ in 0..1_000 {
        call();
    }
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        call();
    }
    let nanos = start.elapsed().as_nanos() / u128::from(ITERATIONS);
    println!("    {label:<28} {nanos:>6} ns/call");
}

fn surfaces(label: &str, view: &SnapshotView, h: &Handles) {
    println!("{label}");
    time("evaluate (entry)", || {
        black_box(view.evaluate(black_box(&h.fast_path)));
    });
    time("evaluate (governed flag)", || {
        black_box(view.evaluate(black_box(&h.new_checkout)));
    });
    time("switch", || {
        black_box(view.switch(black_box(&h.checkout_halt)));
    });
    time("gate_switch", || {
        let _ = black_box(gate_switch(view, black_box(&h.checkout_halt)));
    });
    let pin = view.pin(&h.fast_path).unwrap();
    time("resolve_pinned (honoured)", || {
        let _ = black_box(view.resolve_pinned(black_box(&h.fast_path), black_box(&pin)));
    });
}

#[test]
#[ignore = "a micro-benchmark: run it with --ignored --nocapture"]
fn evaluation_cost() {
    let (catalog, h) = world();
    let resolved = base(catalog, &h.serving, &[Literal::Flag("fast_path", "on")]);

    // No dispatcher current when the view was captured: the event is dropped.
    surfaces("no dispatcher", &views(&resolved, &h), &h);

    // A dispatcher that filters the event out.
    let (filtered, received) = dispatch(LevelFilter::WARN);
    let view = tracing::dispatcher::with_default(&filtered, || views(&resolved, &h));
    surfaces("dispatcher at WARN (filtered)", &view, &h);
    assert_eq!(received.load(Ordering::Relaxed), 0, "nothing reached it");

    // A dispatcher that receives every event, without recording anything.
    let (receiving, received) = dispatch(LevelFilter::DEBUG);
    let view = tracing::dispatcher::with_default(&receiving, || views(&resolved, &h));
    surfaces("dispatcher at DEBUG (counting)", &view, &h);
    assert!(received.load(Ordering::Relaxed) > 0, "events were received");
}
