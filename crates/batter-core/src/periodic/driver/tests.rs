use super::*;
use crate::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
};

#[tokio::test]
async fn initialization_expiry_before_admission_retains_a_zero_run_count() {
    let allowance = Duration::from_secs(1);
    let mut supervisor = Supervisor::new(
        ShutdownBudget::new(
            allowance,
            allowance,
            allowance,
            CleanupBudget::new(allowance, allowance, allowance).unwrap(),
        )
        .unwrap(),
    );
    let history = History::new();
    let retained = history.clone();
    supervisor
        .register("maintenance", move |startup| async move {
            // Exercise the terminal boundary directly: no run was admitted when
            // the initialization allowance expired. Avoid timing a tiny real-clock
            // allowance, which would make this diagnostic assertion nondeterministic.
            finish(
                "maintenance",
                &retained,
                0,
                Ending::InitializationExpired,
                Some(startup),
                None,
            )
        })
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    assert!(!report.is_success());
    let summary = history.reader("maintenance").snapshot();
    assert_eq!(summary.invocations, 0);
    assert_eq!(
        summary.completion,
        PeriodicCompletion::InitializationExpired
    );
    let terminal = summary.terminal_failure.unwrap();
    assert_eq!(terminal.invocation, 0);
    assert_eq!(
        terminal
            .error
            .downcast_ref::<PeriodicInitializationExpired>()
            .unwrap()
            .name,
        "maintenance"
    );
}
