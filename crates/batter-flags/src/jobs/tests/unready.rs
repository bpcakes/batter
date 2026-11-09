//! Ungated jobs before the first publication.

use super::*;

/// An ungated job before the first publication under `unready_until_loaded`
/// has no state to run on: it is deferred by its recheck, as a gated job is,
/// and used to run on the declared defaults the process was not ready to
/// serve. Once a snapshot is published it runs.
#[tokio::test]
async fn an_ungated_job_defers_before_publication_when_the_policy_withholds_readiness() {
    let (catalog, worker, _halt) = world();
    let base = BaseBuilder::new(catalog, worker).unwrap().finish();
    let scope = base.validation_scope();
    let (mut writer, reader) = Reader::live(
        base,
        PrePublication::Unready,
        Duration::from_secs(100),
        Instant::now(),
    );
    let flags = Flags::new(reader);
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = flags.flagged(Probe(calls.clone()), recheck());
    assert_retry(invoke(&handler).await.unwrap_err(), Duration::from_secs(30));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    writer.read_at(Candidate::new(&scope, Version::new(1)), Instant::now());
    invoke(&handler).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
