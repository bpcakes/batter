use std::{io, thread::JoinHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComponentMode {
    Managed,
    Direct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SignalConfiguration {
    Configured,
    Default,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SignalExpectation {
    Survives,
    Terminates,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildInput {
    Piped,
    Null,
}

pub(super) fn join_capture(name: &str, capture: JoinHandle<io::Result<String>>) -> String {
    match capture.join() {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => format!("<{name} capture failed: {error}>"),
        Err(_) => format!("<{name} capture panicked>"),
    }
}
