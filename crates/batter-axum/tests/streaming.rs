//! A response body owned by the library after its handler returned.
//!
//! Failure scenarios from the `batter-9wig.2` assessment and their tests:
//! - a context escaped from `BodyScope` after the body ends:
//!   `lifetime::a_context_kept_beyond_the_body_observes_its_end_as_cancellation`;
//! - a stream that ignores drain:
//!   `shutdown::forced_cancellation_ends_a_stream_that_ignores_every_signal`;
//! - an idle stream past its bound:
//!   `bounds::a_source_that_stops_producing_is_ended_at_the_idle_bound`;
//! - the total deadline during a pending frame:
//!   `bounds::a_source_that_never_stops_is_ended_at_the_total_bound`;
//! - forced cancellation with a pending frame and with a stalled reader,
//!   ordered against the forced connection close:
//!   `sockets::a_pending_owned_body_ends_itself_at_forced_cancellation_before_the_forced_close`
//!   and `sockets::a_stalled_reader_is_ended_by_the_forced_close_and_observed_as_cancelled`;
//! - a client that disconnects mid-stream:
//!   `sockets::a_peer_that_goes_away_destroys_the_body_which_is_observed_as_dropped`;
//! - a handler panic after the hand-over, and inside the builder:
//!   `destruction::a_handler_that_panics_after_the_hand_over_is_observed_as_panicked`
//!   and `destruction::a_panicking_builder_still_cancels_the_scope_it_received_and_is_observed`;
//! - a never-polled body:
//!   `lifetime::a_body_dropped_mid_stream_or_never_polled_is_observed_once_and_cancels_its_scope`;
//! - a scoped, non-global subscriber:
//!   `lifetime::polling_and_destruction_keep_the_handlers_subscriber_and_span`.
//!
//! Beyond that list, a second or late body start for one request is refused
//! in `lifetime::one_body_starts_per_request_and_only_during_response_construction`.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "../../../test-support/capture.rs"]
mod capture;

mod streaming {
    mod bounds;
    mod destruction;
    mod lifetime;
    mod shutdown;
    mod sockets;
    mod support;
}
