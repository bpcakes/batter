use super::{
    admission, escalation, families,
    launch::Config,
    support::{Case, Choices},
    workload,
};
use std::{
    io::{self, Write},
    time::Duration,
};

pub fn child(config: Config) {
    let Config {
        workers,
        seed,
        schedule,
    } = config;
    if schedule == "unjoined-delayed-start" {
        let case = Case {
            workers,
            seed: seed.expect("named fixtures require a seed"),
            index: 0,
            family: "startup",
        };
        case.event("delay-begin");
        std::thread::sleep(Duration::from_secs(2));
        case.event("delay-end");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let result =
            tokio::time::timeout(Duration::from_secs(120), run(workers, seed, &schedule)).await;
        assert!(result.is_ok(), "profile deadline exceeded");
    });
    drop(runtime);
    eprintln!("PROFILE_OK v1 workers={workers} seed={seed:?} schedule={schedule}");
}

async fn run(workers: usize, seed: Option<u64>, schedule: &str) {
    if schedule == "early-exit" {
        std::process::exit(0);
    }
    if schedule == "overflow" {
        overflow_fixture();
    }
    let seeds: Vec<_> = seed.map_or_else(|| (0..32).collect(), |seed| vec![seed]);
    let mut cycles = 0;
    let mut completed = 0;
    let mut coverage = [0; 8];
    for seed in seeds {
        let case = Case {
            workers,
            seed,
            index: 0,
            family: "capacity",
        };
        if schedule == "stuck" {
            blocked_runtime(case).await;
            return;
        }
        if matches!(schedule, "unjoined" | "unjoined-delayed-start") {
            case.bounded(escalation::unjoined_finite(case)).await;
            return;
        }
        match schedule {
            "corpus" => {
                let mut choices = Choices::new(seed);
                for (family, count) in coverage.iter_mut().enumerate() {
                    let (done_cycles, done_tasks) = explore(case, family, &mut choices).await;
                    cycles += done_cycles;
                    completed += done_tasks;
                    *count += 1;
                }
            }
            "capacity-before-drain" => case.bounded(admission::capacity_one(case, false)).await,
            "capacity-after-drain" => case.bounded(admission::capacity_one(case, true)).await,
            _ => panic!("unknown schedule"),
        }
    }
    if schedule == "corpus" {
        let expected_cycles = if seed.is_some() { 2 } else { 64 };
        assert_eq!(cycles, expected_cycles);
        assert_eq!(completed, expected_cycles * 64);
        assert_eq!(
            coverage,
            [expected_cycles / 2; 8],
            "required scenario family missing"
        );
        eprintln!(
            "COUNTS workers={workers} workload_cycles={cycles} completed={completed} families={coverage:?}"
        );
    }
}

fn overflow_fixture() -> ! {
    eprintln!("PROFILE_OK fixture-overflow");
    let mut output = io::stderr().lock();
    for _ in 0..257 {
        output.write_all(&[b'x'; 4096]).unwrap();
    }
    output.flush().unwrap();
    std::process::exit(0);
}

async fn blocked_runtime(case: Case) {
    let timer = async {
        case.event("timer-armed");
        tokio::time::sleep(Duration::from_millis(50)).await;
        case.event("timer-finished");
    };
    let blocker = async {
        case.event("runtime-blocked");
        // Block the thread polling block_on after its short timer was polled.
        // Worker threads may drive time, but cannot poll this parent future.
        loop {
            std::thread::park();
        }
    };
    tokio::join!(biased; timer, blocker);
}

async fn explore(case: Case, family: usize, choices: &mut Choices) -> (u64, u64) {
    let names = [
        "capacity",
        "admission",
        "operation",
        "readiness",
        "failure",
        "ownership",
        "escalation",
        "workload",
    ];
    let case = Case {
        family: names[family],
        ..case
    };
    case.bounded(async {
        match family {
            0 => families::capacity(case).await,
            1 => families::admission(case, choices).await,
            2 => families::operations(case, choices).await,
            3 => families::readiness(case, choices).await,
            4 => families::failures(case, choices).await,
            5 => families::ownership(case, choices).await,
            6 => families::escalation(case, choices).await,
            7 => {
                let mut tasks = 0;
                for index in 0..2 {
                    tasks += workload::cycle(Case { index, ..case }, choices).await;
                }
                return (2, tasks);
            }
            _ => unreachable!(),
        }
        (0, 0)
    })
    .await
}
