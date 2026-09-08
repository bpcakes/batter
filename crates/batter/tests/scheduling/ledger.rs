use batter::lifecycle::{ProcessAdmissionError, ProcessHandle, ProcessReceipt, ProcessScope};
use std::{
    convert::Infallible,
    future::Future,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Default)]
struct Entry {
    accepted: bool,
    started: bool,
    returned: bool,
}

struct State {
    entries: [Entry; 128],
    high: usize,
    rejections: usize,
}

#[derive(Clone)]
pub struct Ledger {
    state: Arc<Mutex<State>>,
    capacity: usize,
}

pub enum Submitter<'a> {
    Root(&'a ProcessHandle),
    Child(&'a ProcessScope),
}

impl Ledger {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: [Entry::default(); 128],
                high: 0,
                rejections: 0,
            })),
            capacity,
        }
    }

    pub fn submit<F, Fut>(
        &self,
        via: Submitter<'_>,
        id: usize,
        factory: F,
    ) -> Result<ProcessReceipt<usize, Infallible>, ProcessAdmissionError>
    where
        F: FnOnce(ProcessScope) -> Fut + Send + 'static,
        Fut: Future<Output = usize> + Send + 'static,
    {
        let mut state = self.state.lock().unwrap();
        assert!(id < state.entries.len());
        assert!(!state.entries[id].accepted, "duplicate accepted ID");
        let ledger = self.clone();
        let factory = move |scope| {
            ledger.started(id);
            async move {
                let value = factory(scope).await;
                assert_eq!(value, id, "wrong task result identity");
                ledger.returned(id);
                Ok::<_, Infallible>(value)
            }
        };
        let result = match via {
            Submitter::Root(process) => process.try_spawn("stress-root", factory),
            Submitter::Child(scope) => scope.try_spawn("stress-child", factory),
        };
        if result.is_ok() {
            state.entries[id].accepted = true;
            let active = state
                .entries
                .iter()
                .filter(|e| e.accepted && !e.returned)
                .count();
            assert!(
                active <= self.capacity,
                "accepted work exceeded shared capacity"
            );
            state.high = state.high.max(active);
        } else {
            state.rejections += 1;
        }
        result
    }

    fn started(&self, id: usize) {
        let mut state = self.state.lock().unwrap();
        let entry = &mut state.entries[id];
        assert!(
            entry.accepted && !entry.started,
            "rejected or duplicate factory invoked"
        );
        entry.started = true;
    }

    fn returned(&self, id: usize) {
        let mut state = self.state.lock().unwrap();
        let entry = &mut state.entries[id];
        assert!(
            entry.started && !entry.returned,
            "unexpected task completion"
        );
        entry.returned = true;
    }

    pub fn active(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.accepted && !e.returned)
            .count()
    }

    pub fn finished(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.returned)
            .count()
    }

    pub fn verify(&self, expected: usize) {
        let state = self.state.lock().unwrap();
        assert_eq!(
            state.entries.iter().filter(|e| e.accepted).count(),
            expected
        );
        assert_eq!(
            state.entries.iter().filter(|e| e.returned).count(),
            expected
        );
        assert!(
            state
                .entries
                .iter()
                .all(|e| e.accepted == e.started && e.started == e.returned)
        );
        assert_eq!(state.high, self.capacity, "capacity was never saturated");
        assert!(
            state.rejections >= 16,
            "required capacity rejection paths not reached"
        );
    }
}
