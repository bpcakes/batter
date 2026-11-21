//! Structured capture of tracing events under an isolated dispatcher.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use tracing::{
    Level,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer, Registry,
    filter::LevelFilter,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};

/// One captured event: what a subscriber is told about it.
#[derive(Clone, Debug)]
pub struct Event {
    pub level: Level,
    pub target: String,
    pub name: String,
    /// Every recorded field as text.
    pub fields: BTreeMap<String, String>,
    /// The metadata's declared field names, recorded or not.
    pub declared: Vec<String>,
    /// The name of the span the event was emitted in, if any.
    pub parent: Option<String>,
}

impl Event {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// Everything the event says, as one string, for absence checks.
    pub fn everything(&self) -> String {
        format!("{self:?}")
    }
}

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

struct Recorder(Arc<Mutex<Vec<Event>>>);

impl<S> Layer<S> for Recorder
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, context: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        event.record(&mut Fields(&mut fields));
        let metadata = event.metadata();
        let captured = Event {
            level: *metadata.level(),
            target: metadata.target().to_owned(),
            name: metadata.name().to_owned(),
            fields,
            declared: metadata
                .fields()
                .iter()
                .map(|field| field.name().to_owned())
                .collect(),
            parent: context.event_span(event).map(|span| span.name().to_owned()),
        };
        self.0.lock().unwrap().push(captured);
    }
}

/// An isolated dispatcher recording the events at or above a level.
pub struct Capture {
    dispatch: tracing::Dispatch,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Capture {
    pub fn new() -> Self {
        Self::at(LevelFilter::TRACE)
    }

    pub fn at(maximum: LevelFilter) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder(Arc::clone(&events)).with_filter(maximum);
        Self {
            dispatch: crate::test_dispatch::new(Registry::default().with(recorder)),
            events,
        }
    }

    /// Runs `body` with this dispatcher current.
    pub fn within<R>(&self, body: impl FnOnce() -> R) -> R {
        tracing::dispatcher::with_default(&self.dispatch, body)
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}
