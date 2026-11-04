//! Serving an assembled boundary over an application-owned TLS listener.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "tls_serving/serving.rs"]
mod serving;
#[path = "tls_serving/support.rs"]
mod support;
