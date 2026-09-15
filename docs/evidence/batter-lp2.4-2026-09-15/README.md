# Fresh consumer acceptance for batter-lp2.4

This directory preserves the bounded fresh-agent evaluation executed on
2026-09-15 against explicitly authorized disposable PostgreSQL 18.4 endpoints.
It is source and local acceptance evidence, not a deployment or population
reliability claim.

The evaluator froze `packet.md`, `modification.md`, and `oracle/check.py` before
their applicable phases. Fresh author `/root/fresh_consumer_author` (Codex based
on GPT-5) received only the public packet. It submitted the initial source before
any compiler or runtime feedback. The first actual compilation retained two
`E0282` inference errors. Repair 1 added explicit result types and passed all
initial oracles. Fresh reviewer `/root/fresh_packet_reviewer` (Codex based on
GPT-5) then found that its named cleanup line relied only on aggregate success.
The author's second and final permitted initial repair required the actual
`database.close` record and passed the complete initial oracle again.

The same author submitted the modification execution-free. Its first submission
compiled and passed normal SIGTERM, later native-failure, and injected
startup-failure scenarios with report-derived `secondary.close` then
`database.close` LIFO records. It added no signal path. The independent reviewer
checked the exact modification prompt and final initial repair, then reported the
packet implementable and compliant with no substantive finding.

Every source submission and its pre-compilation SHA-256 are retained under
`submissions/`. `results.md` records sanitized compilation/oracle outcomes and
binary identities. Lockfiles and build products are excluded. The disposable
credentials, endpoint coordinates, and database contents are not recorded.
