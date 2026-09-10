# Private repository test machinery

`process/` contains std-only Unix launch, output capture, timing and kill/reap
mechanics shared by integration-test binaries. It is not a Cargo package or a
public Batter API. The root remains a virtual workspace with no test target.

Callers supply a `WaitPolicy`; shared code knows no application scenario names,
package-relative assets, lifecycle APIs, or adapter types. Foundation wrappers
in `crates/batter/tests/non_yielding/` include these sources and attach their own
white-box controls. That suite alone owns the non-yielding fixture and Linux
Python parent-death probe. HTTP tests import only these shared mechanics and
provide their own fixture dispatcher and fixed deadline.

Keep controls attached explicitly by the owning suite. Do not put nested test
module inclusions here: every consumer is already compiled with `cfg(test)`, so
that flag cannot distinguish consumers. Do not resolve a suite's assets using
the consuming package's environment from a shared module.

The workspace-relative location remains stable when foundation tests move to a
different same-depth crate. Cargo fmt follows the HTTP module paths; Cargo/Jig
workspace tests compile both consumers, and Jig's `**/*.rs` inputs include these
sources. The foundation controls continue to test the same included implementation.
