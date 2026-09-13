use super::super::work::{Work, WorkMap};
use super::{ParameterObject, database::ParameterCatalog};

/// Logical catalog visits and name lookups, independent of hashing schedules.
/// The observer has no state or work in non-test builds.
#[derive(Clone, Debug, Default)]
pub(super) struct ParameterWork {
    #[cfg(test)]
    operations: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

impl ParameterWork {
    #[cfg(test)]
    pub(super) fn counted(operations: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        Self {
            operations: Some(operations),
        }
    }

    pub(super) fn record(&self) {
        #[cfg(test)]
        if let Some(operations) = &self.operations {
            operations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Work for ParameterWork {
    fn record(&self) {
        self.record();
    }
}

impl Work for &ParameterCatalog {
    fn record(&self) {
        self.record_work();
    }
}

pub(super) struct ParameterIndex<'a> {
    names: WorkMap<&'a str, &'a ParameterObject, &'a ParameterCatalog>,
}

pub(super) fn index_parameters(parameters: &ParameterCatalog) -> ParameterIndex<'_> {
    let mut names = WorkMap::with_capacity(parameters.len(), parameters);
    // Catalog iteration records every visited object, including repeated builds.
    for parameter in parameters.iter() {
        names.insert(parameter.name.as_str(), parameter);
    }
    debug_assert_eq!(names.len(), parameters.len());
    ParameterIndex { names }
}

impl ParameterIndex<'_> {
    pub(super) fn get(&self, name: &str) -> Option<&ParameterObject> {
        self.names.get(name).copied()
    }
}
