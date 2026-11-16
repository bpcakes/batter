use std::panic::{AssertUnwindSafe, catch_unwind};

/// Do not run application payload destructors after an observation failed.
/// Strings have known destruction behavior; opaque payloads must be retained
/// without destruction because even catching their Drop could produce another
/// opaque payload, or block forever. No panic hook is installed or suppressed.
pub(crate) fn isolate(callback: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(callback))
        && !payload.is::<String>()
        && !payload.is::<&'static str>()
    {
        std::mem::forget(payload);
    }
}
