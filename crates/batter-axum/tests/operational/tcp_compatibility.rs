use axum::Router;
use batter_axum::{
    AssembledHttp, register_http, register_http_in, register_http_with_connect_info_in,
};
use batter_core::{RegistrationError, lifecycle::Supervisor};
use tokio::net::TcpListener;

#[test]
fn tcp_registration_preserves_explicit_target_type_arguments() {
    type Register =
        fn(&mut Supervisor, &'static str, TcpListener, Router) -> Result<(), RegistrationError>;
    type RegisterAssembled = fn(
        AssembledHttp,
        &mut Supervisor,
        &'static str,
        TcpListener,
    ) -> Result<(), RegistrationError>;

    // These are the public TCP signatures before generic listeners were added.
    // Inferred calls alone do not catch an added named type parameter: existing
    // callers may explicitly select just the registration authority.
    let _: Register = register_http;
    let _: Register = register_http_in::<Supervisor>;
    let _: Register = register_http_with_connect_info_in::<Supervisor>;
    let _: RegisterAssembled = AssembledHttp::register_in::<Supervisor>;
    let _: RegisterAssembled = AssembledHttp::register_with_connect_info_in::<Supervisor>;
}
