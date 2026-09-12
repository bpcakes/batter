Owning Bead: batter-vef. User-authorized follow-up fixes to batter-gi4.

Capture command interruption at its final poll before future destruction;
distinguish native stop before initialization; update the reference README.
Preserve all existing working changes and the index; no commits.

Seven deterministic boundary tests now cover final-poll versus destruction
cancellation/expiry and native stop/settlement versus process drain. Four defect
controls failed before the behavior fixes; all 20 command and 22 managed tests
pass after repair. Contracts, status, changelog and validation are updated.

Both exact toolchain verify scripts and five HTTP smokes per toolchain pass.
Final Jig required gates pass with fresh api:test receipt
receipt_01M2A2QYY9F7CTHW7A3F31V0JP. Source guards retain identical Batter/native
execution inputs throughout both acceptance and Jig. Evidence, command logs and
scope limitations are recorded in docs/validation.md. The final implementation
inspection and git diff whitespace check pass. Jig source scopes already cover
the new test module; no contract input changes are needed.

Classification remains a local implementation correction. Managed readiness
keeps its existing post-destruction check; completed commands now match their
documented final-poll boundary. Cleanup eligibility and native ownership remain
unchanged. The new Stopped variant requires exhaustive consumers to add an arm.

Tracker batter-vef is closed. The final metadata policy refresh passed, reusing
the unchanged Rust evidence. Work is complete without staging or committing.
