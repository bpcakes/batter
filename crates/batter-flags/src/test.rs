//! Test flags: the fixture path for handlers, jobs and components under test.
//!
//! Tests build a catalog's values directly and hand them to the surface under
//! test, without a settings source or a control plane. This is the fixture
//! path; the escape hatch in `low_level` is not.
//!
//! Scaffold: the fixture type arrives with the declared-mode task.
