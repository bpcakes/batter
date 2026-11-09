#!/usr/bin/env python3
"""Compile the real private attempt module against ownership/privacy controls.

The clock and diagnostic shims supply dependency names only. No fixture runs;
real Tokio/HTTP behavior is covered by the crate's independent runtime tests.
"""

import json
from pathlib import Path
import tempfile

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parents[1]
ATTEMPT = ROOT / "crates/batter-otlp/src/pipeline/attempt.rs"
PREFIX = '''#![allow(dead_code)]
extern crate self as tokio;
pub mod time {
    pub use std::time::Instant;
    pub async fn timeout_at<F: Future>(_: Instant, future: F) -> Result<F::Output, ()> {
        Ok(future.await)
    }
}
mod diagnostics {
    #[derive(Clone, Copy)]
    pub enum ExportFailure { Encoding, HeadersTimedOut, BodyTimedOut }
    #[derive(Clone, Copy)]
    pub enum ExportOutcome { Acknowledged, Failed(ExportFailure) }
}
mod transport {
    struct Acknowledged;
'''
POSITIVE = '''
fn proof(preparing: attempt::Preparing, sending: attempt::Sending,
         receiving: attempt::Receiving, owner: &mut attempt::Owner) {
    let _ = preparing.send();
    let _ = sending.receive();
    let _ = receiving.complete(Ok(Acknowledged));
    let _ = owner.run(crate::time::Instant::now(), || async {});
}
'''
CASES = [
    ("positive", POSITIVE, None),
    ("reuse-preparing", '''fn proof(p: attempt::Preparing) {
        let _ = p.send(); let _ = p.send(); }''', "E0382"),
    ("reuse-receiving", '''fn proof(r: attempt::Receiving) {
        let _ = r.complete(Ok(Acknowledged)); let _ = r.complete(Ok(Acknowledged)); }''', "E0382"),
    ("clone-permission", '''fn proof(p: attempt::Preparing) { let _ = p.clone(); }''', "E0599"),
    ("premature-acknowledgement", '''fn proof(p: attempt::Preparing) {
        let _ = p.complete(Ok(Acknowledged)); }''', "E0599"),
    ("fabricate-observation", '''fn proof() {
        let _ = attempt::State::Completed(crate::diagnostics::ExportOutcome::Acknowledged); }''', "E0603"),
    ("overlap-owners", '''fn proof(o: &mut attempt::Owner) {
        let a = o.run(crate::time::Instant::now(), || async {});
        let b = o.run(crate::time::Instant::now(), || async {});
        drop((a, b)); }''', "E0499"),
    ("paired-begin", '''fn proof(o: &attempt::Owner) {
        o.begin(crate::time::Instant::now()); }''', "E0599"),
]


def main():
    with tempfile.TemporaryDirectory(prefix="batter-attempt-types-") as directory:
        root = Path(directory)
        for name, snippet, expected in CASES:
            source = root / f"{name}.rs"
            source.write_text(PREFIX + f"#[path = {json.dumps(str(ATTEMPT))}] mod attempt;\n"
                              + snippet + "\n}\n")
            outcome, = run_parallel(
                [["rustc", "--edition=2024", "--crate-type=lib", "--crate-name=attempt_control",
                  "--emit=metadata", "--error-format=json", str(source), "-o", str(root / "check.rmeta")]],
                cwd=ROOT, timeout=30, output_limit=128 * 1024,
            )
            codes = []
            for line in outcome.stderr.splitlines():
                message = json.loads(line)
                if message.get("level") == "error":
                    codes.append((message.get("code") or {}).get("code"))
            settled = (outcome.reaped and outcome.output_eof and not outcome.watchdog
                       and not outcome.overflow and not outcome.errors)
            valid = outcome.ok if expected is None else (
                settled and outcome.status == 1 and expected in codes
                and all(code in (expected, None) for code in codes)
            )
            if not valid:
                render_outcomes([name], [outcome])
                raise RuntimeError(f"unexpected ownership control result: {name}")
            print(f"PASS: {name}")


if __name__ == "__main__":
    main()
