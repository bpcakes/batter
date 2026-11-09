//! Pin outcomes are read inside a native invocation, not a fabricated JobFlags.
use super::*;
use batter_flags_core::{CivilDate, Expiry, PinRejection, Pinned, PinnedResolution, Toggle};
struct Check<F>(F);
#[async_trait]
impl<F: Fn(&JobFlags) + Send + Sync> FlagAwareHandler for Check<F> {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("test.pins")
    }
    async fn execute(
        &self,
        _: JobExecution<'_>,
        flags: &JobFlags,
        _: Value,
    ) -> Result<JobCompletion, JobFailure> {
        (self.0)(flags);
        Ok(JobCompletion::success())
    }
}
fn pin_world() -> (
    Catalog,
    ProcessKind,
    ProcessKind,
    ProcessKind,
    KillSwitch,
    batter_flags_core::ModeFlag<Toggle>,
    batter_flags_core::ReleaseFlag<Toggle>,
) {
    let (mut b, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let worker = b.process_kind("worker").unwrap();
    let other = b.process_kind("other").unwrap();
    let halt = b
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let mode = b.mode_flag("mode", Toggle::Off).declare().unwrap();
    let release = b
        .release_flag(
            "release",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 1, 1).unwrap()),
        )
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .governed_by(&halt)
        .declare()
        .unwrap();
    (b.build(), serving, worker, other, halt, mode, release)
}

#[tokio::test]
async fn pins_round_trip_cross_kind_override_refusal_and_explicit_fallback() {
    let (catalog, serving, _, _, _, mode, release) = pin_world();
    let source = TestFlags::builder(catalog, serving)
        .unwrap()
        .set(&mode, Toggle::On)
        .set(&release, Toggle::On)
        .build()
        .unwrap();
    let mode_pin: Pinned<Toggle> =
        serde_json::from_value(serde_json::to_value(source.view().pin(&mode).unwrap()).unwrap())
            .unwrap();
    let pin = source.view().pin(&release).unwrap();
    let (catalog, _, worker, _, _, mode, release) = pin_world();
    let worker_flags = TestFlags::builder(catalog, worker)
        .unwrap()
        .build()
        .unwrap();
    invoke(&worker_flags.flags().flagged(Check(|flags: &JobFlags| {
        assert!(matches!(flags.resolve_pinned(&mode, &mode_pin), PinnedResolution::Honoured(d) if d.value() == Toggle::On));
        assert!(matches!(flags.resolve_pinned(&release, &pin), PinnedResolution::Honoured(d) if d.value() == Toggle::On));
        for because in [UnpinnedBecause::ScheduledJob, UnpinnedBecause::PayloadPredatesPinning] {
            assert_eq!(flags.evaluate_unpinned(&mode, because).value(), Toggle::Off);
        }
        match flags.resolve_pinned(&mode, &pin) {
            PinnedResolution::Incompatible(refusal) => {
                assert_eq!(refusal.rejection(), PinRejection::KeyMismatch);
                assert_eq!(refusal.fall_back_to_live().value(), Toggle::Off);
            }
            _ => panic!("wrong key honoured"),
        }
    }), recheck())).await.unwrap();
    let (catalog, _, worker, _, halt, _, release) = pin_world();
    let tripped = TestFlags::builder(catalog, worker)
        .unwrap()
        .engage(&halt)
        .build()
        .unwrap();
    invoke(&tripped.flags().flagged(Check(|flags: &JobFlags| {
        assert!(matches!(flags.resolve_pinned(&release, &pin), PinnedResolution::Overridden {decision, by} if decision.value() == Toggle::Off && by.as_str() == "halt"));
    }), recheck())).await.unwrap();
    let (catalog, _, _, other, _, _, release) = pin_world();
    let foreign_kind = TestFlags::builder(catalog, other).unwrap().build().unwrap();
    invoke(&foreign_kind.flags().flagged(
        Check(|flags: &JobFlags| {
            assert_eq!(
                flags.resolve_pinned(&release, &pin).rejection(),
                Some(PinRejection::NotOwned)
            );
        }),
        recheck(),
    ))
    .await
    .unwrap();
    let (catalog, _, worker, _, _, mode, _) = pin_world();
    let base = BaseBuilder::new(catalog, worker).unwrap().finish();
    let declared = Flags::new(Reader::declared(base));
    invoke(&declared.flagged(
        Check(|flags: &JobFlags| {
            assert_eq!(
                flags.resolve_pinned(&mode, &mode_pin).rejection(),
                Some(PinRejection::TestProvenance)
            );
        }),
        recheck(),
    ))
    .await
    .unwrap();
}
