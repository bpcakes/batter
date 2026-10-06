//! Shared capture assertions; offline controls stay in the `profile` test target.
use std::sync::{Arc, Mutex};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context};

type Result = std::result::Result<(), batter_core::BoxError>;

#[derive(Clone, Default)]
pub(super) struct HookEvents(Arc<Mutex<Vec<String>>>);

impl HookEvents {
    pub(super) fn assert_redacted(&self, hook: &str, markers: &[&str]) -> Result {
        let observed = self.0.lock().unwrap().join("\n");
        require(
            !markers.iter().any(|marker| observed.contains(marker)),
            "SQLx pool logging disclosed native error contents",
        )?;
        self.assert_hook_redacted(hook, markers)
    }

    pub(super) fn assert_hook_redacted(&self, hook: &str, markers: &[&str]) -> Result {
        let observed = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.contains(hook))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        require(
            !observed.is_empty(),
            "expected SQLx hook error was not logged",
        )?;
        require(
            !markers.iter().any(|marker| observed.contains(marker)),
            "SQLx hook logging disclosed native error contents",
        )?;
        require(
            observed.contains("PostgreSQL operation failed"),
            "hook log omitted safe diagnostic",
        )
    }
}

impl<S: Subscriber> Layer<S> for HookEvents {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target().contains("pool") {
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }
}

struct Fields(String);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        write!(self.0, " {}={value:?}", field.name()).unwrap();
    }
}

fn require(condition: bool, message: &'static str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(std::io::Error::other(message).into())
    }
}
