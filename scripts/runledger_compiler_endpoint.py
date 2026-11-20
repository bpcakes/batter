"""Bind live SQLx preparation to the PostgreSQL settings checked by psql."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile


# Build scripts are host programs launched by Cargo, including on cross builds.
# Consult Cargo for the merged settings without invoking a target runner.
BUILD_SCRIPT = """\
use std::{path::Path, process::Command};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ok = Command::new(root.join("python"))
        .args(["-I", "-S"]).arg(root.join("observe.py")).arg("observe")
        .current_dir(root).status().expect("Cargo settings observer").success();
    assert!(ok, "Cargo settings observer failed");
}
"""


def endpoint_signature(salt):
    actual = os.environb.get(b"DATABASE_URL")
    digest = hashlib.sha256(salt + actual).hexdigest() if actual is not None else None
    return sorted(name for name in os.environ if name.startswith("PG")), digest


def observe():
    if any(os.environ.get(name) for name in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")):
        Path("wrapped").touch()
        return
    # An opaque compiler or cache can bypass a check placed outside it.
    # Use Cargo's selected native rustc directly after checking each invocation.
    selected = shutil.which(os.environ["RUSTC"])
    sysroot = subprocess.check_output([selected, "--print", "sysroot"]).rstrip(b"\n")
    compiler = Path(os.fsdecode(sysroot)) / "bin" / "rustc"
    rustup = shutil.which("rustup")
    if not (compiler.is_file() and (os.path.samefile(selected, compiler)
                                    or (rustup and os.path.samefile(selected, rustup)))):
        Path("wrapped").touch()
        return
    names, digest = endpoint_signature(Path("salt").read_bytes())
    Path("answer").write_text(json.dumps([names, digest, str(compiler)]), encoding="ascii")


def refresh_build_root(workspace):
    """Ask Cargo for the original workspace's effective intermediate build directory."""
    from parallel_process import run_parallel

    result, = run_parallel([["cargo", "metadata", "--format-version=1", "--no-deps", "--offline", "--locked"]],
                           cwd=workspace, timeout=60, output_limit=8 * 1024 * 1024)
    if not result.ok:
        raise ValueError("could not resolve the Cargo workspace build directory")
    # A conventional target subtree also keeps refresh artifacts out of source
    # copies when the configured build directory lives inside the checkout.
    build = Path(json.loads(result.stdout)["build_directory"]) / "runledger-prepare" / "target"
    build.mkdir(parents=True, exist_ok=True)
    return build


def cargo_build_config(directory):
    # Cargo can configure intermediate artifacts independently of --target-dir.
    return "build.build-dir=" + json.dumps(str(directory), ensure_ascii=False)


def cargo_compiler_endpoint(directory, database, build):
    from parallel_process import run_parallel

    with tempfile.TemporaryDirectory(dir=build, prefix="batter-cargo-env-probe-") as scratch:
        probe = Path(scratch)
        (probe / "src").mkdir()
        (probe / "Cargo.toml").write_text(
            '[package]\nname = "cargo-env-probe"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n')
        (probe / "build.rs").write_text(BUILD_SCRIPT)
        (probe / "src/lib.rs").touch()
        (probe / "python").symlink_to(sys.executable)
        (probe / "observe.py").symlink_to(Path(__file__).resolve())
        salt = os.urandom(32)
        (probe / "salt").write_bytes(salt)
        expected = hashlib.sha256(salt + database.encode("utf-8")).hexdigest()
        command = ["cargo", "check", "--lib", "--quiet", "--offline",
                   "--config", cargo_build_config(probe / "target"), "--target-dir", str(probe / "target"),
                   "--manifest-path", str(probe / "Cargo.toml")]
        result, = run_parallel([command], cwd=directory, timeout=300, output_limit=64 * 1024)
        failure = "could not ask Cargo which [env] settings apply"
        if (probe / "wrapped").exists():
            raise ValueError("live SQLx preparation requires a wrapper-free Cargo compiler")
        if not result.ok:
            raise ValueError(failure)
        try:
            names, digest, compiler = json.loads((probe / "answer").read_bytes())
        except (OSError, ValueError):
            raise ValueError(failure) from None
        if not isinstance(names, list) or any(not re.fullmatch(r"PG[A-Za-z0-9_]*", name) for name in names):
            raise ValueError(failure)
        return names, digest == expected, compiler


@contextmanager
def guard_preparation(compiler, database, packages, build):
    """Check the actual Cargo-provided environment immediately before exec of rustc."""
    with tempfile.TemporaryDirectory(dir=build, prefix="batter-prepare-compiler-") as scratch:
        root = Path(scratch)
        salt = os.urandom(32)
        (root / "settings").write_text(json.dumps({
            "rustc": compiler, "packages": packages, "salt": salt.hex(),
            "digest": hashlib.sha256(salt + database.encode("utf-8")).hexdigest(),
        }), encoding="ascii")
        launcher = root / "rustc"
        # Both paths may contain spaces. The command contains no credentials.
        command = [sys.executable, "-I", "-S", str(Path(__file__).resolve()), "guard", str(root)]
        launcher.write_text('#!/bin/sh\nexec ' + shlex.join(command) + ' "$@"\n')
        launcher.chmod(0o700)
        config = root / "compiler.toml"
        path = json.dumps(str(launcher), ensure_ascii=False)
        output = json.dumps(str(build), ensure_ascii=False)
        config.write_text(
            f'[build]\ntarget-dir = {output}\nbuild-dir = {output}\nrustc = {path}\nrustc-wrapper = ""\nrustc-workspace-wrapper = ""\n'
            f'[env]\nRUSTC = {{ value = {path}, force = true }}\n'
            'RUSTC_WRAPPER = { value = "", force = true }\n'
            'RUSTC_WORKSPACE_WRAPPER = { value = "", force = true }\n', encoding="utf-8")
        yield launcher, config
        if any(not (root / package).is_file() for package in packages):
            raise ValueError("Cargo did not run the checked compiler for every query package")


def compile_checked(root, args):
    settings = json.loads((root / "settings").read_bytes())
    names, digest = endpoint_signature(bytes.fromhex(settings["salt"]))
    if names or digest != settings["digest"]:
        sys.exit("refusing a compiler endpoint different from the PostgreSQL settings checked by psql")
    package = os.environ.get("CARGO_PKG_NAME")
    if package in settings["packages"] and "--crate-name" in args:
        name = args[args.index("--crate-name") + 1]
        library = any(a == "--crate-type" and b in ("lib", "rlib") for a, b in zip(args, args[1:]))
        metadata = any(a.startswith("--emit=") and "metadata" in a[7:].split(",") for a in args)
        if name == package.replace("-", "_") and library and metadata:
            (root / package).touch()
    os.execv(settings["rustc"], [settings["rustc"], *args])


if __name__ == "__main__":
    if sys.argv[1] == "observe":
        observe()
    elif sys.argv[1] == "guard":
        compile_checked(Path(sys.argv[2]), sys.argv[3:])
