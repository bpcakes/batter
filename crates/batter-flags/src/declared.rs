//! Declared mode: values resolved once at startup from the application's
//! settings source.
//!
//! Strict literals, closed-state absence, process-kind ownership, retired
//! names and development-only restrictions are enforced here with value-free
//! diagnostics; a value changes by changing its setting and restarting.
//!
//! Scaffold: the resolver and its outcomes arrive with the declared-mode task.
