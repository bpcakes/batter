# Outbound owned-connection API comparison

Owning Bead: `batter-u387`. Compared baseline
`e94c591ec25a2adb94e5310ae2ad68c791510a0a` with the working tree using the
same installed nightly and the method in
[the public-API evidence guide](public-api-diff-2026-10-09/README.md):

```sh
cargo +nightly rustdoc -p batter-outbound-http --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  <baseline-doc-dir> <working-doc-dir> batter-outbound-http
```

Executed result: 197 entries before and after, zero removed, zero added.
The tool does not compare signatures; manual diff inspection confirms unchanged
public constructors, draft methods, send/send_in signatures and bounds, enum
variants, accessors, reexports and constants. The 0.0.1 baseline predates this
package; this comparison covers the complete previously checked-in package surface.

Internal reqwest request/client values are replaced by private native HTTP
requests, immutable rustls configuration and an owned connection driver. No
public request/outcome path or call shape changes. No Breaking entry,
Migration rustdoc or deprecated shim is required. A pooled compatibility path
would restore the timing gap and is not provided. Behavioral changes are
recorded in CHANGELOG: fresh connections and phase-derived pre-handoff outcomes.
