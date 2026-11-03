#![cfg(target_os = "linux")]
//! Client construction against the host certificate store, on Linux.
//!
//! The operating system verifier for HTTPS origins reads the host store, which
//! on Linux follows `SSL_CERT_FILE` and `SSL_CERT_DIR`. The parent runs this
//! test again in a child whose store is empty, so no other test sees it.

use batter_outbound_http::{Client, ClientError, ConnectTimeout, Origin};
use std::{net::SocketAddr, process::Command, time::Duration};

const TEST: &str = "an_empty_trust_store_refuses_https_clients_but_not_loopback_ones";
const CHILD: &str = "BATTER_OUTBOUND_TRUST_STORE_CHILD";

fn connect_timeout() -> ConnectTimeout {
    ConnectTimeout::new(Duration::from_secs(1)).expect("a valid timeout")
}

#[test]
fn an_empty_trust_store_refuses_https_clients_but_not_loopback_ones() {
    if std::env::var_os(CHILD).is_some() {
        return child();
    }
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("SSL_CERT_FILE", "/dev/null")
        .env_remove("SSL_CERT_DIR")
        .output()
        .expect("child started");
    assert!(output.status.success(), "child failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed"), "child ran no test: {stdout}");
}

fn child() {
    let origin = Origin::https("https://provider.example").expect("an https origin");
    let refused = Client::new(origin, connect_timeout());
    assert_eq!(refused.as_ref().err(), Some(&ClientError::Build));
    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert_eq!(message, "the outbound HTTP client could not be built");
    // A loopback origin is plain HTTP and never reads the host store.
    let local = SocketAddr::from(([127, 0, 0, 1], 9));
    let loopback = Origin::loopback(local).expect("a loopback origin");
    assert!(Client::new(loopback, connect_timeout()).is_ok());
}
