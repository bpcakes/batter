//! The production worker with metrics export enabled.

use super::{
    BIND_LIMIT, Command, ExecutableChild, ExecutableKind, REAP_LIMIT, SeparateChild, Signal,
    SignalRequest, Stdio, configure_provider_worker, io, listener,
};
use std::time::Duration;

/// The fixed export schedule may settle one in-flight periodic attempt (3 s)
/// and then spend the whole final allowance (5 s) after the service stops.
const EXPORT_SETTLEMENT: Duration = Duration::from_secs(8);

impl ExecutableChild {
    /// Start the production provider worker exporting to `collector`.
    pub fn start_exporting_worker(
        endpoint: &str,
        provider_base_url: &str,
        worker_id: &str,
        collector: &str,
    ) -> io::Result<Self> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"));
        configure_provider_worker(
            &mut command,
            endpoint,
            "127.0.0.1:0",
            provider_base_url,
            worker_id,
            "1",
            "1",
        );
        command
            .env("BATTER_METRICS_OTLP_ENDPOINT", collector)
            .stdin(Stdio::null());
        let announcement = listener::Announcement::new(&mut command)?;
        let mut child = SeparateChild::spawn("production exporting worker", command, None)?;
        let listener = child
            .wait_for_listener(&announcement, BIND_LIMIT)
            .map_err(io::Error::other)?;
        Ok(Self {
            child,
            kind: ExecutableKind::ProductionRunningSuccess,
            listener: Some(listener),
        })
    }

    /// Require clean running shutdown with silent streams, allowing the bounded
    /// diagnostic settlement after service completion.
    pub fn stop_exporting(mut self, signal: Signal, forbidden: &[&str]) -> Result<(), String> {
        let limit = REAP_LIMIT + EXPORT_SETTLEMENT;
        if self.child.signal(signal)? != SignalRequest::Accepted {
            let run = self.child.wait(limit)?;
            run.check(forbidden)?;
            return Err(format!(
                "{} exited before the {signal:?} request",
                run.label
            ));
        }
        self.child.wait(limit)?.production_running_child(forbidden)
    }
}
