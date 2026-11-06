# References: Build, verification and repository tooling

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Cargo test artifacts and profiles, 2026-09-24

- The [Cargo build-cache reference](https://doc.rust-lang.org/cargo/reference/build-cache.html)
  documents target directory selection through environment, configuration and
  command-line flags. The facade runner reads Cargo metadata's resolved directory
  and passes an absolute compiler-specific child directory to every consumer.
- The [profile override reference](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)
  scopes settings to a package in the workspace root manifest. Its generic-code
  discussion explains why levels 0 and 1 can reuse instantiations from other
  crates, whereas levels 2 and 3 avoid that sharing. The at-rest test override
  uses level 2 with debug assertions and overflow checks explicitly enabled;
  the detached leaf manifest retains its existing default profile.
- Cargo's [artifact message reference](https://doc.rust-lang.org/cargo/reference/external-tools.html#artifact-messages)
  documents `--message-format=json` and the `compiler-artifact.executable` path,
  including fresh cached artifacts. The HTTP gate consumes that path instead of
  assuming the default target directory. Rechecked 2026-09-24.

## Rust minimum toolchain: reviewed 2026-11-02

- Rust's [1.98.1 release notes](https://github.com/rust-lang/rust/blob/stable/RELEASES.md#version-1981-2026-09-03)
  list one fix, a rustc vtable miscompilation
  ([rust-lang/rust#161441](https://github.com/rust-lang/rust/issues/161441)).
  Packages therefore declare `rust-version = "1.98.1"`, the exact pinned and
  CI-tested release, rather than `1.98`. Cargo accepted the three-component
  value for all four Clippy configurations and the complete verification.

## GitHub Actions CI scope selection: reviewed 2026-11-02

- [Swatinem/rust-cache at the pinned commit](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/action.yml)
  declares the `cache-hit` output as an exact key match. Its
  [restore](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/restore.ts)
  sets that output only when the restored key equals the computed key, and its
  [save](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/save.ts)
  skips an up-to-date cache. A verified-tree push therefore skips tests only
  when the master cache already matches the current lockfile and compiler.
- [actions/upload-artifact v7.0.2](https://github.com/actions/upload-artifact/blob/cf430e030ddbb5b0abf93d22962f4752f3646cd9/action.yml),
  pinned at `cf430e030ddbb5b0abf93d22962f4752f3646cd9`, accepts `name`, `path`,
  `if-no-files-found`, `retention-days` and `overwrite` and runs on `node24`.
- GitHub's [artifacts REST API](https://docs.github.com/en/rest/actions/artifacts#list-artifacts-for-a-repository)
  lists repository artifacts filtered by exact `name`; each artifact's
  `workflow_run` carries `repository_id` and `head_repository_id`. Fork runs can
  upload artifacts under any name, so the planner trusts only markers whose two
  IDs match. It then reads the [workflow run](https://docs.github.com/en/rest/actions/workflow-runs#get-a-workflow-run)
  and requires `path` `.github/workflows/ci.yml` and a `pull_request` or
  `merge_group` event. A live call on 2026-11-02 returned
  `.github/workflows/ci.yml` and `pull_request` for an existing PR run.
- GitHub's [pull_request event](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request)
  checks out the PR's test merge commit. The planner diffs that commit against
  its first parent, the base tip, and diffs pushes from the event's `before`.
- GitHub's [status check functions](https://docs.github.com/en/actions/reference/workflows-and-actions/expressions#status-check-functions)
  and [needs context](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts#needs-context)
  define `!cancelled()` and dependency results. Test jobs use them to run after
  a failed planner, and the recording job uses them to require that every test
  job succeeded, or that Runlimit PostgreSQL was skipped by the plan.

## Workspace packaging: reviewed 2026-09-08

The package split in [ADR-006](../adr/006-workspace-packages.md) follows Cargo's
documented package/dependency boundaries:

- [Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html): a
  virtual manifest has no root package; members share a lockfile and output
  directory. Package metadata inheritance is opt-in, so package versions and
  Rust minimums can remain explicit.
- [Dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html#features):
  building multiple workspace members unifies their dependency features. Run
  the foundation check separately to verify its isolated dependency selection.
- [Development dependency cycles](https://doc.rust-lang.org/cargo/reference/resolver.html#dev-dependency-cycles):
  Cargo permits some cycles involving tests, but unit-test builds can expose
  incompatible copies of library types. Keep generic test support independent.
- [Rust-version resolution](https://doc.rust-lang.org/cargo/reference/resolver.html#rust-version):
  mixed-minimum workspaces use resolution heuristics, not a guarantee of each
  member's compatibility. Batter pins and verifies its declared Rust 1.98.1 minimum.

These rules do not supply execution evidence. Database provisioning remains external.

## Jig prebuilt release runtime: 2026-10-04

The [Jig 0.7.1 release](https://github.com/bpcakes/jig-sh/releases/tag/v0.7.1)
publishes x86_64 and ARM64 archives for Linux and macOS, each with a SHA-256 file.
The tagged [runtime pin documentation](https://github.com/bpcakes/jig-sh/blob/v0.7.1/docs/configuration.md#runtime-release-pins)
and [generated installer](https://github.com/bpcakes/jig-sh/blob/v0.7.1/templates/project/scripts/install-jig.sh.jinja)
define `.jig/runtime-version` independently of template provenance. Batter copies
that installer from commit `e1f60d1ce6c49c4a64b9c091972a2596ef47b78e` while
keeping its project-authored file-budget-only launcher and catalogs.

The installer reuses an exact-version, contract-compatible executable or downloads
the official archive and checksum over HTTPS. It verifies SHA-256, a single regular
native executable, version and contract/profile support before publication under
the cache lock. Linux assets require glibc 2.35 or newer; macOS assets target macOS
13 or newer. Unsupported targets, missing curl and missing archives retain the
upstream crates.io source fallback; download, checksum and compatibility failures
do not fall back. CI caches only the executable in ordinary-checkout and linked-
worktree cache locations. No Cargo metadata, source stamps or lock files are cached.
