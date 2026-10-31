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
