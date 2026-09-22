Owning Bead: `batter-mlk`.

Preserve v1 wire format, public API, key derivation and MAC bytes. Verify existing
merge resolution; defer body copy until complete decode; clarify trust state,
authoritative context, fenced rotation and licensing; add focused failure
regressions, packaged external consumer and feature graph checks. Validate both
Rust toolchains, HTTP smoke and downstream fixtures/API without changing the
downstream checkout.

Progress: the handed-off merge has no unmerged index entries or conflict markers
in the previously conflicted paths; locked offline Cargo metadata succeeds.
The decoder allocation reorder is the only production behavior change. Public
signatures, accepted bytes, cryptographic operations, key derivation and MAC
concatenation are unchanged. Context authority, revision freshness and persistence
fencing require application state unavailable to the synchronous leaf; no new
public API or remote-effect proof is introduced.

Executed locally on Linux: 40 leaf unit tests, one public API test and two
doctests on Rust 1.98.1; complete standalone gate on exact Rust 1.94.0, including
the new separate unpacked-artifact consumer. An external temporary consumer
compiled the downstream configuration crate against this leaf with both default
and test-support features, decoded/opened its existing envelope fixture and
verified byte-identical same-key rewrap plus unchanged MAC segment concatenation.
The downstream baseline was 3a76e786045d12a9e4083e452b81121968754a00 and its worktree
remained clean. This is a focused compatibility probe, not a full downstream
application or production migration test.

Tracker merge repair: import initially failed semantic verification because 26
comment identifiers collided across eight merged issues. A temporary build of the
available newer Beads source diagnosed duplicates explicitly. A copy-based import
validated the repair first; only colliding numeric IDs were reassigned, with every
other issue/comment field compared for equality. The installed `br` then imported
all 175 source issues successfully, retained the new owning task, and exported
normally. No tracker records or comment contents were removed.

Both complete workspace verification scripts passed on Rust 1.98.1 and 1.94.0,
including native PostgreSQL suites, Clippy and rustdoc. All five HTTP smoke modes
and 12 HTTP example tests passed on each toolchain. The final standalone package
gate passed again after the last leaf edit. Lock SHA-256:
`eaade179cdf3b407bd021d90915a429a19fe801c9931d939e6ebe40c970a1469`.

The leaf README is compiled into rustdoc. Added it to the Clippy/test/test-locked
input scopes in both Jig contract files and to the input-freshness regression.
The first in-flight Jig pass correctly refused to certify a mid-run contract
change; final receipts use the updated scopes. Final diff review found no conflict
markers or unmerged entries. Optional remote wrapping, a new format,
application storage protocols, production key-policy/nonce accounting and new
identity framing remain outside this compatibility-preserving PR repair.

Final required Jig verification passed with fresh Clippy, formatting, tests,
contract and file-budget evidence. The executed `api:test` receipt is
`receipt_01M3461P8VXXA5SZ3BJP91DF6F`; target validation is
`receipt_01M3461T39WGJJY5EARAXDTF25`. Both evidence and gates report passed/fresh.
One earlier stable-input workspace run exited 101; its retained CLI/receipt output
was truncated before the failing case, so the cause was not established. The full
workspace diagnostic rerun and final complete Jig test matrix passed without
source changes. This does not establish a root cause for the earlier failure.
The final retry retained complete JSON diagnostics. All 11 Jig integration tests
and the final compiled-README freshness regression also passed. Hosted CI and
macOS execution for this repair remain unverified.
