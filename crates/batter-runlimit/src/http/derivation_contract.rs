//! A selector cannot replace the boundary deriver's policy or hasher before
//! deriving: both fields of `SubjectDeriver` are private, so assigning either
//! fails to compile, whichever field a change might expose.
//!
//! ```compile_fail,E0616
//! use batter_runlimit::{ConsumptionError, Quota};
//! use batter_runlimit::http::{AuthInput, DirectPeer, HttpQuota};
//! use runlimit_core::{KeyHasher, Limiter};
//!
//! fn replaces_the_policy<L: Limiter + 'static>(
//!     quota: Quota<L>,
//!     policies: Vec<L::Policy>,
//!     other_policy: &'static L::Policy,
//! ) where
//!     L::CheckAllError: ConsumptionError,
//! {
//!     let hasher = KeyHasher::new([7; 32]).unwrap(); // Deterministic test secret only.
//!     let _ = HttpQuota::new(
//!         quota, policies, hasher,
//!         |_input: AuthInput| async { Ok::<&'static str, std::io::Error>("owner") },
//!         move |principal: &&'static str, _peer: DirectPeer, mut subject| {
//!             subject.policy = other_policy;
//!             subject.derive(*principal)
//!         },
//!     );
//! }
//! ```
//!
//! ```compile_fail,E0616
//! use batter_runlimit::{ConsumptionError, Quota};
//! use batter_runlimit::http::{AuthInput, DirectPeer, HttpQuota};
//! use runlimit_core::{KeyHasher, Limiter};
//!
//! fn replaces_the_hasher<L: Limiter + 'static>(quota: Quota<L>, policies: Vec<L::Policy>)
//! where
//!     L::CheckAllError: ConsumptionError,
//! {
//!     let hasher = KeyHasher::new([7; 32]).unwrap(); // Deterministic test secret only.
//!     // Deterministic test secret only.
//!     let other_hasher: &'static KeyHasher = Box::leak(Box::new(KeyHasher::new([8; 32]).unwrap()));
//!     let _ = HttpQuota::new(
//!         quota, policies, hasher,
//!         |_input: AuthInput| async { Ok::<&'static str, std::io::Error>("owner") },
//!         move |principal: &&'static str, _peer: DirectPeer, mut subject| {
//!             subject.hasher = other_hasher;
//!             subject.derive(*principal)
//!         },
//!     );
//! }
//! ```
