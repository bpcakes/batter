use super::*;

/// Offline checks for capture setup and already-completed process ownership.
pub fn process_contracts() {
    let (stdout_reader, stdout_writer) = io::pipe().unwrap();
    let (stderr_reader, stderr_writer) = io::pipe().unwrap();
    let mut command = Command::new("/bin/true");
    command.stdout(stdout_writer).stderr(stderr_writer);
    let mut starts = 0;
    let error = match start_captures(command, stdout_reader, stderr_reader, |reader| {
        starts += 1;
        if starts == 2 {
            Err(io::Error::other("injected second capture failure"))
        } else {
            Capture::start(reader)
        }
    }) {
        Ok(_) => panic!("the injected second capture failure must be returned"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "injected second capture failure");

    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf 'batter-fixture:completed-child\\n'"]);
    let mut child = SeparateChild::spawn("completed child", command, None).unwrap();
    assert!(child.child.wait().unwrap().success());
    child
        .wait_event("completed-child", REAP_LIMIT)
        .expect("event remains available after the child is reaped");
    assert_eq!(
        child.signal(Signal::Term).unwrap(),
        SignalRequest::AlreadyExited,
        "a completed child is distinguished from an accepted signal request"
    );
    let run = child.wait(REAP_LIMIT).unwrap();
    assert!(run.status.success());
    run.stdout.validate(&["completed-child"], &[]).unwrap();
    run.stderr.validate(&[], &[]).unwrap();

    // The old executable oracle accepted this exact status/diagnostic pair.
    // Without a process-local signal witness it must now be rejected.
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "printf 'Error: reference service failed\\n' >&2; exit 1",
    ]);
    let run = SeparateChild::spawn("unrelated failure", command, None)
        .unwrap()
        .wait(REAP_LIMIT)
        .unwrap();
    assert!(run.signal_fixture_child(Signal::Term, &[]).is_err());

    running_child_report_contract();

    let private = "postgres://fixture:private@localhost/database";
    let mut output = Output::default();
    output.record(
        format!("{private}\nthread panicked at private boundary\n").as_bytes(),
        Instant::now(),
    );
    let diagnostic = safe_check(&output, "private output").unwrap_err();
    assert!(!diagnostic.contains(private));
    assert!(!format!("{:?}", SafeOutput(&output)).contains(private));

    let provider_url = "http://provider.invalid/";
    let forbidden = super::super::provider_forbidden_values(private, provider_url);
    for credential in [
        super::super::AUTH_TOKEN,
        super::super::super::provider::TOKEN,
    ] {
        let mut command = Command::new("/usr/bin/printf");
        command.args(["%s\n", credential]);
        let run = SeparateChild::spawn("credential output", command, None)
            .unwrap()
            .wait(REAP_LIMIT)
            .unwrap();
        let diagnostic = run.check_streams(&forbidden).unwrap_err();
        assert!(!diagnostic.contains(credential));
    }

    for (scenario, signal) in [
        ("signal.broadcast.term", Signal::Term),
        ("signal.broadcast.int", Signal::Int),
    ] {
        let mut child = StartupChild::start(scenario, private).unwrap();
        child
            .wait_event(super::super::SIGNAL_LISTENERS_READY, REAP_LIMIT)
            .unwrap();
        let mut child = child.request_stop(signal, &[private]).unwrap();
        child
            .wait_event(super::super::SIGNAL_OBSERVED, REAP_LIMIT)
            .unwrap();
        child
            .finish(
                &[
                    super::super::SIGNAL_LISTENERS_READY,
                    super::super::SIGNAL_OBSERVED,
                ],
                &[private],
            )
            .unwrap();
    }
}

fn running_child_report_contract() {
    let address: SocketAddr = "127.0.0.1:43127".parse().unwrap();
    assert_eq!(
        super::listener::parse(address.to_string().as_bytes()).unwrap(),
        address
    );
    for invalid in [
        "",
        "127.0.0.1:0",
        "0.0.0.0:1234",
        "private",
        "127.0.0.1:12\nprivate",
    ] {
        assert!(super::listener::parse(invalid.as_bytes()).is_err());
    }
    let mut silent = Command::new("/bin/sh");
    silent.args(["-c", "exit 0"]);
    let run = SeparateChild::spawn("clean running shutdown", silent, None)
        .unwrap()
        .wait(REAP_LIMIT)
        .unwrap();
    run.production_running_child(&[])
        .expect("discovery never uses stdout");

    let mut command = Command::new("/usr/bin/printf");
    command.args(["%s\n", &address.to_string()]);
    let run = SeparateChild::spawn("unexpected stdout", command, None)
        .unwrap()
        .wait(REAP_LIMIT)
        .unwrap();
    assert!(run.production_running_child(&[]).is_err());
}
