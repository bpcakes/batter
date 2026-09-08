use std::{
    io::{self, BufRead, Read},
    process,
    sync::mpsc,
    thread,
    time::Duration,
};

pub const CHILD_ARGS: &[&str] = &[
    "--exact",
    "child_fixture",
    "--nocapture",
    "--test-threads=1",
];
pub const PARENT_GONE: i32 = 74;
pub const EMERGENCY_DEADLINE: i32 = 75;
pub const INVALID_LAUNCH: i32 = 76;
pub const EMERGENCY_LIMIT: Duration = Duration::from_secs(10);

pub fn scenario() -> Option<String> {
    // Ambient environment variables are never launch authority. Ordinary
    // Cargo discovery leaves this entry inert, even with a scenario exported.
    if !std::env::args().skip(1).eq(CHILD_ARGS.iter().copied()) {
        return None;
    }
    // Arm before any blocking input or application work. This is deliberately
    // an OS thread, independent of both Tokio and the parent test process.
    thread::spawn(|| {
        thread::sleep(EMERGENCY_LIMIT);
        process::exit(EMERGENCY_DEADLINE);
    });
    let (authorized, received) = mpsc::channel();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut command = String::new();
        if (&mut input).take(256).read_line(&mut command).is_err() {
            process::exit(INVALID_LAUNCH);
        }
        let mut fields = command.split_whitespace();
        let valid = fields.next() == Some("batter-fixture-v1")
            && fields.next().and_then(|pid| pid.parse::<u32>().ok()) == Some(process::id());
        let scenario = fields.next().map(str::to_owned);
        if !valid || scenario.is_none() || fields.next().is_some() || !command.ends_with('\n') {
            process::exit(INVALID_LAUNCH);
        }
        authorized.send(scenario.unwrap()).unwrap();
        // The parent retains the only writer. EOF on parent death (or any
        // unexpected further input/error) ends the whole process, without
        // yielding the blocked task or pretending its finalizers executed.
        let mut unexpected = [0];
        loop {
            match input.read(&mut unexpected) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                _ => process::exit(PARENT_GONE),
            }
        }
    });
    Some(received.recv().expect("fixture launch reader stopped"))
}
