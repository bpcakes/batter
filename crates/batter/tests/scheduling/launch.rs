use std::{
    io::{self, BufRead, Read},
    process,
    sync::mpsc,
    thread,
    time::Duration,
};

const CHILD_ARGS: &[&str] = &[
    "--exact",
    "scheduling_child",
    "--nocapture",
    "--test-threads=1",
];
const STARTUP_LIMIT: Duration = Duration::from_secs(5);
const EMERGENCY_LIMIT: Duration = Duration::from_secs(149);
const _: () = assert!(EMERGENCY_LIMIT.as_secs() > 140 + 5);
const INVALID_LAUNCH: i32 = 86;

pub struct Config {
    pub workers: usize,
    pub seed: Option<u64>,
    pub schedule: String,
}

pub fn receive() -> Option<Config> {
    // An ambient variable is never launch authority. Ordinary test discovery
    // stays inert, even when a previous command exported a fixture flag.
    if !std::env::args().skip(1).eq(CHILD_ARGS.iter().copied()) {
        return None;
    }
    // Arm before any stdin read. External observation is capped at 140 seconds
    // plus five seconds to reap/drain, before this emergency process exit.
    thread::spawn(|| {
        thread::sleep(EMERGENCY_LIMIT);
        process::exit(88);
    });
    let (authorized, received) = mpsc::channel();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut line = String::new();
        if (&mut input).take(128).read_line(&mut line).is_err() {
            process::exit(INVALID_LAUNCH);
        }
        let config = parse(&line).unwrap_or_else(|| process::exit(INVALID_LAUNCH));
        if authorized.send(config).is_err() {
            process::exit(INVALID_LAUNCH);
        }
        // Keep this same reader: it may have buffered bytes beyond the record.
        // EOF or unexpected further input means the parent no longer owns the
        // launch pipe; this exit does not execute application finalizers.
        let mut extra = [0];
        loop {
            match input.read(&mut extra) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                _ => process::exit(87),
            }
        }
    });
    Some(
        received
            .recv_timeout(STARTUP_LIMIT)
            .unwrap_or_else(|_| process::exit(INVALID_LAUNCH)),
    )
}

fn parse(line: &str) -> Option<Config> {
    if line.len() >= 128 || !line.ends_with('\n') {
        return None;
    }
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 5
        || fields[0] != "batter-scheduling-v1"
        || fields[1].parse::<u32>().ok()? != process::id()
    {
        return None;
    }
    let workers = fields[2].parse().ok()?;
    if ![2, 4].contains(&workers) {
        return None;
    }
    let seed = if fields[3] == "all" {
        None
    } else {
        Some(fields[3].parse().ok()?)
    };
    let schedule = fields[4];
    if !matches!(
        schedule,
        "corpus"
            | "capacity-before-drain"
            | "capacity-after-drain"
            | "stuck"
            | "unjoined"
            | "unjoined-delayed-start"
            | "early-exit"
            | "overflow"
    ) || (schedule != "corpus" && seed.is_none())
    {
        return None;
    }
    Some(Config {
        workers,
        seed,
        schedule: schedule.to_owned(),
    })
}
