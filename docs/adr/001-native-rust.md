# ADR-001: Native Rust, not an Effect port

Status: accepted for the MVP source. Date: 2026-09-07.

Context: the user's Rust backend stack already has strong typing, Result,
ownership, Tokio futures, Axum state, and SQLx. Effect v4's useful inspiration
is coordinated runtime behavior, not recreating TypeScript's missing language
facilities inside Rust.

Decision: expose ordinary futures, explicit constructors, concrete application
error enums, and native runtime/database/web types. Do not introduce a generic
Effect<A,E,R>, universal AppContext/service locator, repository abstraction,
custom scheduler, or requirement to express all business methods as Tower services.

Consequences: service construction is explicit and dependency requirements are
not automatically union-inferred as in Effect. The API remains familiar and
small. Lifecycle/cleanup contracts are narrower than a full effect runtime and
must be documented accurately. A DI graph can only be reconsidered after two
consumers demonstrate a concrete need and cancellation/resource semantics are
proven, not because a feature-parity chart contains an empty cell.
