## Progress
- Inspect installation, pending files, fresh-checkout behavior, and existing CI.
- Remove unused generated integration pieces, preserve Rust failure assertions, and validate.

## Surprises & Discoveries
The initial doctor passes using a local cached runtime; embedded source metadata has no portable installation pin. Generated CI duplicates the existing Rust matrix.

## Decision Log
Keep the repository launcher, MCP, Rust checks, file budgets, and durable work memory. Remove unrelated language configuration and local adoption backup metadata from the commit.

## Outcomes & Retrospective
Pending final verification. Run doctor, contract and guide checks, both Rust toolchains, HTTP smoke, package inspection, and Git whitespace checks. Do not commit or publish.

## Progress update
Completed footprint cleanup, both Rust verification matrices, three HTTP smoke scenarios, archive inclusion/exclusion fixture, workflow syntax checks, contract/guide checks, and MCP initialization/discovery/inspection.

## Decision Log update
The owner requires Jig 0.3.0 without an exact revision. Removed the intermediate development source pin. Contract-v7 upstream does not provide version-only enforcement; release-tag adoption still records a resolved commit. Preserve the original embedded source metadata and report the installation policy as unresolved.

## Outcomes & Retrospective update
Local checks pass. A final-configuration fresh-checkout probe fails without a compatible cache, so the task is not ready to commit and this plan remains open. See docs/validation.md. No commit or publication performed.

## Final installation decision
The owner accepts release selection by tag and the resulting source revision until first-class version pinning exists. Selected official v0.3.0 through jig update --vcs-ref v0.3.0 in a disposable checkout; resolved SHA 8629700b92cd9ab8b09f8ff86de4fc1573469c83 matches the remote tag. Preserved the trimmed repository footprint. Fresh release installation with no cache or overrides passed; doctor needs no initialized vault. The release launcher and installer match byte-for-byte. Documentation directs revision-preserving updates through update --recopy and explains that ordinary update advances upstream. The earlier installation-policy blocker is resolved by this accepted interim policy.