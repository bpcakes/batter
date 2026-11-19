"""Observe Cargo's PostgreSQL settings during procedural macro expansion."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile


# This crate has no dependencies. The integration test invokes its macro in a
# target compilation, even when the host and target differ. Only the proc macro
# and our Python observer execute, on the host. Cargo never runs a target.
MACRO = """\
extern crate proc_macro;
use std::{path::Path, process::Command};

#[proc_macro]
pub fn observe(_: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ok = Command::new(root.join("python"))
        .args(["-I", "-S"]).arg(root.join("observe.py")).current_dir(root)
        .status().expect("compiler observer").success();
    assert!(ok, "compiler observer failed");
    proc_macro::TokenStream::new()
}
"""


def observe():
    # SQLx macros read the environment of this rustc. No env!(), Cargo
    # directives, compiler diagnostics or generated source carry the URL.
    salt = Path("salt").read_bytes()
    actual = os.environb.get(b"DATABASE_URL")
    digest = hashlib.sha256(salt + actual).hexdigest() if actual is not None else None
    names = sorted(name for name in os.environ if name.startswith("PG"))
    Path("answer").write_text(json.dumps([names, digest]), encoding="ascii")


def cargo_compiler_endpoint(directory, database):
    from parallel_process import run_parallel

    with tempfile.TemporaryDirectory(prefix="batter-cargo-env-probe-") as scratch:
        probe = Path(scratch)
        (probe / "src").mkdir()
        (probe / "tests").mkdir()
        (probe / "Cargo.toml").write_text(
            '[package]\nname = "cargo-env-probe"\nversion = "0.0.0"\nedition = "2021"\n'
            '[lib]\nproc-macro = true\n[workspace]\n')
        (probe / "src/lib.rs").write_text(MACRO)
        (probe / "tests/endpoint.rs").write_text('cargo_env_probe::observe!();\n')
        (probe / "python").symlink_to(sys.executable)
        (probe / "observe.py").symlink_to(Path(__file__).resolve())
        salt = os.urandom(32)
        (probe / "salt").write_bytes(salt)
        expected = hashlib.sha256(salt + database.encode("utf-8")).hexdigest()
        command = ["cargo", "check", "--test", "endpoint", "--quiet", "--offline",
                   "--manifest-path", str(probe / "Cargo.toml"), "--target-dir", str(probe / "target")]
        result, = run_parallel([command], cwd=directory, timeout=300, output_limit=64 * 1024)
        failure = "could not ask Cargo which [env] settings apply"
        if not result.ok:
            raise ValueError(failure)
        try:
            names, digest = json.loads((probe / "answer").read_bytes())
        except (OSError, ValueError):
            raise ValueError(failure) from None
        if not isinstance(names, list) or any(not re.fullmatch(r"PG[A-Za-z0-9_]*", name) for name in names):
            raise ValueError(failure)
        return names, digest == expected


if __name__ == "__main__":
    observe()
