// Suite-specific signal controls on the existing private Unix process owner.
#[allow(dead_code)]
#[path = "../../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../../test-support/process/evidence.rs"]
mod evidence;
#[allow(dead_code)]
#[path = "../../../../test-support/process/launch.rs"]
pub mod launch;
#[allow(dead_code)]
#[path = "../../../../test-support/process/timing.rs"]
mod timing;
#[allow(dead_code)]
mod watchdog {
    include!("../../../../test-support/process/watchdog.rs");

    pub struct StartupChild(FixtureChild);
    impl StartupChild {
        pub fn start(stage: &str, endpoint: &str) -> io::Result<Self> {
            Self::start_at(stage, endpoint, "127.0.0.1:0")
        }

        pub fn start_at(stage: &str, endpoint: &str, bind: &str) -> io::Result<Self> {
            let (input, writer) = io::pipe()?;
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args(launch::CHILD_ARGS)
                .stdin(input)
                .env_clear()
                .env("DATABASE_URL", endpoint)
                .env("BATTER_BIND", bind)
                .env("BATTER_POOL_MAX_CONNECTIONS", "1")
                .env("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "20000")
                .env("JOBS_WORKER_ID", "startup-signal")
                .env(
                    "BATTER_AUTH_OWNER_ID",
                    "00000000-0000-0000-0000-000000000001",
                )
                .env("BATTER_AUTH_TOKEN", "fixture-token");
            let mut child = FixtureChild::spawn_command(stage, command, Some(writer))?;
            writeln!(
                child.parent_pipe.as_mut().unwrap(),
                "batter-fixture-v1 {} {stage}",
                child.child.id()
            )?;
            Ok(Self(child))
        }

        pub fn diagnostics(&self) -> String {
            format!("{:?}", self.0.capture.snapshot())
        }

        pub fn terminate(mut self) -> Result<(), String> {
            let sent = Command::new("/bin/kill")
                .args(["-TERM", &self.0.child.id().to_string()])
                .status()
                .map_err(|e| e.to_string())?;
            if !sent.success() {
                return Err("could not send SIGTERM".into());
            }
            let run = self
                .0
                .wait(Duration::from_secs(8))
                .map_err(|e| e.to_string())?;
            run.validate(ExpectedExit::Success, &[], &[])?;
            if !run
                .output
                .text()
                .contains("startup-child:owned-cleanup-complete")
            {
                return Err(format!("missing startup cleanup proof: {run:?}"));
            }
            Ok(())
        }
    }
}
pub use watchdog::StartupChild;

pub fn child(stage: &str) {
    use batter::{
        settings::SettingsSource,
        startup::{StartupCause, StartupError},
    };
    use batter_example_reference_service::{
        config::{ConfigMode, RootSettings},
        runtime::{self, RuntimeStartupFailure},
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let settings =
                RootSettings::from_process(ConfigMode::Serve, None, SettingsSource::default())
                    .unwrap();
            if stage == "production" {
                runtime::run(settings)
                    .await
                    .expect("clean production shutdown");
                println!("startup-child:owned-cleanup-complete");
                return;
            }
            let error = runtime::run(settings)
                .await
                .expect_err("startup must drain");
            let failure = error
                .downcast_ref::<RuntimeStartupFailure>()
                .expect("retained startup failure");
            let StartupError::Failed(startup) = failure.startup() else {
                panic!("startup coordinator failed")
            };
            assert_eq!(startup.stage, stage);
            assert!(matches!(startup.cause, StartupCause::Draining));
            assert!(startup.cleanup.is_success());
            assert!(
                startup
                    .cleanup
                    .records
                    .iter()
                    .any(|record| record.name == "postgres.pool")
            );
            println!("startup-child:owned-cleanup-complete");
        });
}
