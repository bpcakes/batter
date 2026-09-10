//! Test-only workaround for tracing-core 0.1.36's single-dispatch interest cache.
//! See tokio-rs/tracing#2874. Never install a global default subscriber here.
use std::sync::OnceLock;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

pub fn new(subscriber: impl tracing::Subscriber + Send + Sync + 'static) -> tracing::Dispatch {
    // Keep the bootstrap maximum OFF until the real dispatch finishes rebuilding
    // interest with two registrations. NoSubscriber has no max-level hint and
    // would enable macros during the single-dispatch initialization window.
    static SENTINEL: OnceLock<tracing::Dispatch> = OnceLock::new();
    SENTINEL.get_or_init(|| {
        tracing::Dispatch::new(tracing_subscriber::registry().with(LevelFilter::OFF))
    });
    tracing::Dispatch::new(subscriber)
}
