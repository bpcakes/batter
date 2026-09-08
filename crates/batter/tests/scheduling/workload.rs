use super::{
    ledger::{Ledger, Submitter},
    support::{Case, Choices, supervisor, yields},
};
use batter::lifecycle::{ProcessAdmissionError, ProcessHandle, ProcessReceipt, ProcessScope};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::{Semaphore, mpsc};

pub async fn cycle(case: Case, choices: &mut Choices) -> u64 {
    let mut supervisor = supervisor(8);
    let ledger = Ledger::new(8);
    let cleanup_ledger = ledger.clone();
    supervisor
        .on_cleanup("stress-resource", move || async move {
            cleanup_ledger.verify(64);
            Ok(())
        })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    for round in 0..8 {
        round_of_work(case, &process, &ledger, round, choices).await;
    }
    let report = running.shutdown().await.unwrap();
    ledger.verify(64);
    assert!(report.is_success());
    assert!(
        report.tasks.is_empty(),
        "successful history must not accumulate"
    );
    assert_eq!(report.completed_process_tasks, ledger.finished() as u64);
    assert_eq!(report.cleanup.records.len(), 1);
    case.event("64-results-reconciled");
    report.completed_process_tasks
}

async fn round_of_work(
    case: Case,
    process: &ProcessHandle,
    ledger: &Ledger,
    round: usize,
    choices: &mut Choices,
) {
    let gate = Arc::new(Semaphore::new(0));
    let (started_tx, mut started_rx) = mpsc::channel(4);
    let mut receipts = Vec::new();
    for parent in 0..4 {
        let receipt = spawn_parent(
            process,
            ledger,
            round * 8 + parent,
            gate.clone(),
            started_tx.clone(),
            choices.next(),
        )
        .await;
        if choices.next() & 1 == 0 {
            // A receipt has no work/capacity ownership. The ledger and report
            // must still include this task even when no result waiter survives.
            drop(receipt);
        } else {
            receipts.push(receipt);
        }
    }
    let mut scopes = Vec::new();
    for _ in 0..4 {
        scopes.push(started_rx.recv().await.unwrap());
    }
    assert_eq!(
        ledger.active(),
        8,
        "parked roots and children must all consume capacity"
    );
    case.event("roots-and-children-at-capacity");
    for via in [Submitter::Root(process), Submitter::Child(&scopes[0])] {
        let id = 64 + round;
        assert!(matches!(
            ledger.submit(via, id, move |_| async move { id }),
            Err(ProcessAdmissionError::Full)
        ));
    }
    gate.add_permits(4);
    for receipt in receipts {
        receipt.wait().await.unwrap();
    }
    // End-of-factory accounting is not a JoinHandle. Later root retries may
    // still see Full until the wrapper releases its actual permit.
    while ledger.finished() != (round + 1) * 8 {
        tokio::task::yield_now().await;
    }
}

async fn spawn_parent(
    process: &ProcessHandle,
    ledger: &Ledger,
    id: usize,
    gate: Arc<Semaphore>,
    started: mpsc::Sender<ProcessScope>,
    delay: u64,
) -> ProcessReceipt<usize, Infallible> {
    loop {
        let child_ledger = ledger.clone();
        let gate = gate.clone();
        let started = started.clone();
        let result = ledger.submit(Submitter::Root(process), id, move |scope| async move {
            yields(delay).await;
            let child = spawn_child(&child_ledger, &scope, id + 4, gate, started, delay).await;
            assert_eq!(child.wait().await.unwrap(), id + 4);
            id
        });
        match result {
            Ok(receipt) => return receipt,
            Err(ProcessAdmissionError::Full) => tokio::task::yield_now().await,
            Err(_) => panic!("unexpected stress admission rejection"),
        }
    }
}

async fn spawn_child(
    ledger: &Ledger,
    scope: &ProcessScope,
    id: usize,
    gate: Arc<Semaphore>,
    started: mpsc::Sender<ProcessScope>,
    delay: u64,
) -> ProcessReceipt<usize, Infallible> {
    loop {
        let parent_scope = scope.clone();
        let gate = gate.clone();
        let started = started.clone();
        let result = ledger.submit(Submitter::Child(scope), id, move |_| async move {
            started.send(parent_scope).await.unwrap();
            let _permit = gate.acquire().await.unwrap();
            yields(delay.rotate_left(7)).await;
            id
        });
        // Previous-round factories may have returned before their wrappers have
        // released permits. Full is a normal rejection here, not a test retry.
        match result {
            Ok(receipt) => return receipt,
            Err(ProcessAdmissionError::Full) => tokio::task::yield_now().await,
            Err(_) => panic!("unexpected descendant admission rejection"),
        }
    }
}
