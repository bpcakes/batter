
Business rejection, terminal storage failure and uncertain disposition are
different contracts. Broad error enums cannot be put into narrower wrappers.

A business rejection cannot be put in a terminal storage wrapper:

```compile_fail,E0308
use batter_example_reference_service::delivery::{CommandFailure, StorageError};
fn invalid(rejection: CommandFailure) {
    StorageError::Scope(Box::new(batter::sqlx::PgScopeError::Application(rejection)));
}
```

A known rollback cannot be wrapped as uncertainty:

```compile_fail,E0308
use batter_example_reference_service::delivery::{CommandFailure, SubmitResult, UncertainSubmission};
fn invalid(rejection: CommandFailure) {
    UncertainSubmission::Atomic(Box::new(
        batter::sqlx::PgAtomicError::<SubmitResult, _>::Rejected(rejection),
    ));
}
```
