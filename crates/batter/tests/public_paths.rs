use std::any::TypeId;

#[test]
fn facade_reexports_keep_core_type_identity() {
    assert_eq!(
        TypeId::of::<batter::ConfigurationError>(),
        TypeId::of::<batter_core::ConfigurationError>()
    );
    assert_eq!(
        TypeId::of::<batter::RegistrationError>(),
        TypeId::of::<batter_core::RegistrationError>()
    );
}

#[test]
fn established_module_paths_remain_available() {
    let _: Option<batter::operation::Interruption> = None;
    let _: Option<batter::lifecycle::Readiness> = None;
    let _: Option<batter::startup::StartupOutcome<()>> = None;
    let _: Option<batter::PanicPayload> = None;
}
