use std::time::Duration;

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    periodic::PeriodicRun,
};
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, PrePublication, ReleaseFlag, SafeState,
    SettingsPrefix, StaleRule, SwitchState, Toggle, Version,
    low_level::{BaseBuilder, Candidate, CandidateRow, Disposition, ValidationScope},
};
use tokio::time::Instant;

use crate::{
    Flags,
    reader::{Reader, Writer},
};

pub const SECOND: Duration = Duration::from_secs(1);

pub fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            SECOND,
            SECOND,
            SECOND,
            CleanupBudget::new(SECOND, SECOND, SECOND).unwrap(),
        )
        .unwrap(),
    )
}

pub fn success() -> PeriodicRun<std::io::Error> {
    Ok(())
}

pub struct Fixture {
    pub flags: Flags,
    pub writer: Writer,
    pub halt: KillSwitch,
    pub unowned: KillSwitch,
    pub flag: ReleaseFlag<Toggle>,
    scope: ValidationScope,
}

impl Fixture {
    pub fn new(pre_publication: PrePublication) -> Self {
        let (mut catalog, serving, _) =
            CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted")
                .unwrap();
        let worker = catalog.process_kind("worker").unwrap();
        let halt = catalog
            .kill_switch("prune_halt", SafeState::Released, StaleRule::HoldLastKnown)
            .declare()
            .unwrap();
        let unowned = catalog
            .kill_switch("worker_halt", SafeState::Released, StaleRule::HoldLastKnown)
            .owned_only_by(&worker)
            .declare()
            .unwrap();
        let flag = catalog
            .release_flag(
                "fast_path",
                Toggle::Off,
                Expiry::on(CivilDate::new(2027, 3, 31).unwrap()),
            )
            .declare()
            .unwrap();
        let base = BaseBuilder::new(catalog.build(), serving).unwrap().finish();
        let scope = base.validation_scope();
        let (writer, reader) = Reader::live(base, pre_publication, SECOND * 1000, Instant::now());
        Self {
            flags: Flags::new(reader),
            writer,
            halt,
            unowned,
            flag,
            scope,
        }
    }

    pub fn publish(&mut self, version: u64, halt: SwitchState, flag: Toggle) {
        let at = Version::new(version);
        let candidate = Candidate::new(&self.scope, at)
            .row(CandidateRow::switch(&self.halt, halt, at))
            .row(CandidateRow::flag(&self.flag, flag, at));
        assert_eq!(
            self.writer.read_at(candidate, Instant::now()),
            Disposition::Applied
        );
    }
}
