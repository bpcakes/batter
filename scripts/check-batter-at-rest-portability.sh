#!/usr/bin/env bash
set -euo pipefail

readonly REQUIRED_RUST_TOOLCHAIN="1.94.0"
readonly EXPECTED_RUST_VERSION="1.94"
readonly EXPECTED_PACKAGE="batter-at-rest"
readonly EXPECTED_VERSION="0.1.0"
readonly EXPECTED_LICENSE="LicenseRef-CreditKit-Proprietary"

script_dir=$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
crate_source="$repo_root/crates/batter-at-rest"

for required_program in cargo rustc jq git mktemp cp tar awk grep dirname node python3; do
    if ! command -v "$required_program" >/dev/null 2>&1; then
        printf 'batter-at-rest portability: required program is unavailable: %s\n' "$required_program" >&2
        exit 1
    fi
done
if [[ -z "${HOME:-}" ]]; then
    printf 'batter-at-rest portability: required environment variable is unset: HOME\n' >&2
    exit 1
fi
if ! python3 -c 'import tomllib' >/dev/null 2>&1; then
    printf 'batter-at-rest portability: Python 3.11 or newer with tomllib is required\n' >&2
    exit 1
fi

if ! rust_version=$(rustc "+$REQUIRED_RUST_TOOLCHAIN" --version 2>&1); then
    printf 'batter-at-rest portability: install Rust %s with `rustup toolchain install %s --profile minimal`\n' \
        "$REQUIRED_RUST_TOOLCHAIN" "$REQUIRED_RUST_TOOLCHAIN" >&2
    printf '%s\n' "$rust_version" >&2
    exit 1
fi
case "$rust_version" in
    "rustc $REQUIRED_RUST_TOOLCHAIN "*) ;;
    *)
        printf 'batter-at-rest portability: expected Rust %s, got %s\n' \
            "$REQUIRED_RUST_TOOLCHAIN" "$rust_version" >&2
        exit 1
        ;;
esac
printf 'batter-at-rest portability: using %s\n' "$rust_version"
if ! cargo_version=$(cargo "+$REQUIRED_RUST_TOOLCHAIN" --version 2>&1); then
    printf 'batter-at-rest portability: Cargo is unavailable for Rust %s\n' \
        "$REQUIRED_RUST_TOOLCHAIN" >&2
    printf '%s\n' "$cargo_version" >&2
    exit 1
fi
printf 'batter-at-rest portability: using %s\n' "$cargo_version"

temporary_parent=${TMPDIR:-/tmp}
if [[ -z "$temporary_parent" || ! -d "$temporary_parent" ]]; then
    printf 'batter-at-rest portability: temporary parent is unavailable: %s\n' "$temporary_parent" >&2
    exit 1
fi
temporary_parent=$(CDPATH= cd -- "$temporary_parent" && pwd -P)
readonly temporary_parent
if ! portability_root=$(mktemp -d "$temporary_parent/batter-at-rest-portability.XXXXXX"); then
    printf 'batter-at-rest portability: could not create a temporary root under %s\n' \
        "$temporary_parent" >&2
    exit 1
fi
if [[ -z "$portability_root" || ! -d "$portability_root" || "$portability_root" == "/" ]]; then
    printf 'batter-at-rest portability: refusing unsafe temporary root: %s\n' "$portability_root" >&2
    exit 1
fi
portability_root=$(CDPATH= cd -- "$portability_root" && pwd -P)
readonly portability_root

cleanup() {
    rm -rf -- "$portability_root"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

case "$portability_root/" in
    "$repo_root/"*)
        printf 'batter-at-rest portability: temporary root must be outside the checkout: %s\n' \
            "$portability_root" >&2
        exit 1
        ;;
esac

if [[ ! -f "$crate_source/Cargo.toml" ]]; then
    printf 'batter-at-rest portability: missing crate manifest: %s\n' "$crate_source/Cargo.toml" >&2
    exit 1
fi
for source_cargo_config in \
    "$crate_source/.cargo/config.toml" \
    "$crate_source/.cargo/config"; do
    if [[ -e "$source_cargo_config" || -L "$source_cargo_config" ]]; then
        printf 'batter-at-rest portability: crate-local Cargo configuration is forbidden: %s\n' \
            "$source_cargo_config" >&2
        exit 1
    fi
done
controlled_cargo_home="$portability_root/cargo-home"
cargo_launch_dir="$portability_root/cargo-launch"
target_root="$portability_root/targets"
mkdir -p "$controlled_cargo_home" "$cargo_launch_dir" "$target_root"
readonly controlled_cargo_home cargo_launch_dir target_root

assert_no_parent_cargo_config() {
    local current_dir

    current_dir=$(CDPATH= cd -- "$1" && pwd -P)
    while true; do
        if [[ -e "$current_dir/.cargo/config.toml" || -L "$current_dir/.cargo/config.toml" || \
            -e "$current_dir/.cargo/config" || -L "$current_dir/.cargo/config" ]]; then
            printf 'batter-at-rest portability: Cargo configuration exists on isolated launch path: %s/.cargo\n' \
                "$current_dir" >&2
            return 1
        fi
        if [[ "$current_dir" == "/" ]]; then
            break
        fi
        current_dir=${current_dir%/*}
        if [[ -z "$current_dir" ]]; then
            current_dir="/"
        fi
    done
}

assert_no_parent_cargo_config "$cargo_launch_dir"

assert_no_parent_cargo_manifest() {
    local current_dir

    current_dir=$(CDPATH= cd -- "$1" && pwd -P)
    while true; do
        if [[ -e "$current_dir/Cargo.toml" || -L "$current_dir/Cargo.toml" ]]; then
            printf 'batter-at-rest portability: Cargo manifest exists on isolated launch path: %s/Cargo.toml\n' \
                "$current_dir" >&2
            return 1
        fi
        if [[ "$current_dir" == "/" ]]; then
            break
        fi
        current_dir=${current_dir%/*}
        if [[ -z "$current_dir" ]]; then
            current_dir="/"
        fi
    done
}

assert_no_parent_cargo_manifest "$cargo_launch_dir"

cargo_environment=(
    env -i
    "CARGO_HOME=$controlled_cargo_home"
    "HOME=$HOME"
    "PATH=$PATH"
    "RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}"
)
for passthrough_name in \
    ALL_PROXY all_proxy \
    HTTP_PROXY http_proxy \
    HTTPS_PROXY https_proxy \
    NO_PROXY no_proxy \
    SSL_CERT_DIR SSL_CERT_FILE; do
    passthrough_value=${!passthrough_name:-}
    if [[ -n "$passthrough_value" ]]; then
        cargo_environment+=("$passthrough_name=$passthrough_value")
    fi
done
readonly -a cargo_environment

cargo_msrv() {
    local crate_dir=$1
    local target_dir=$2
    local cargo_command=$3
    shift 3

    (
        cd "$cargo_launch_dir"
        "${cargo_environment[@]}" \
            "CARGO_TARGET_DIR=$target_dir" \
            cargo "+$REQUIRED_RUST_TOOLCHAIN" "$cargo_command" \
            --manifest-path "$crate_dir/Cargo.toml" "$@"
    )
}

validate_source_manifest_policy() {
    local manifest_path=$1

    python3 - "$manifest_path" <<'PYTHON'
import sys
import tomllib
from pathlib import Path

manifest_path = Path(sys.argv[1])
try:
    with manifest_path.open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
except (OSError, tomllib.TOMLDecodeError) as error:
    print(
        f"batter-at-rest portability: could not parse source manifest {manifest_path}: {error}",
        file=sys.stderr,
    )
    raise SystemExit(1)


def reject(message: str) -> None:
    print(f"batter-at-rest portability: {message} in {manifest_path}", file=sys.stderr)
    raise SystemExit(1)


for override_table in ("patch", "replace"):
    if override_table in manifest:
        reject("dependency source override tables are forbidden")

package = manifest.get("package")
if not isinstance(package, dict):
    reject("a package table is required")
if "workspace" in package:
    reject("package workspace association is forbidden")
for field, value in package.items():
    if field != "metadata" and isinstance(value, dict) and value.get("workspace") is True:
        reject(f"workspace inheritance is forbidden for package.{field}")


def reject_workspace_dependencies(container: dict, owner: str) -> None:
    for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
        dependencies = container.get(table_name)
        if not isinstance(dependencies, dict):
            continue
        for dependency_name, specification in dependencies.items():
            if isinstance(specification, dict) and specification.get("workspace") is True:
                reject(
                    "workspace inheritance is forbidden for "
                    f"{owner}.{table_name}.{dependency_name}"
                )


reject_workspace_dependencies(manifest, "manifest")
targets = manifest.get("target")
if isinstance(targets, dict):
    for target_name, target in targets.items():
        if isinstance(target, dict):
            reject_workspace_dependencies(target, f"target.{target_name}")

lints = manifest.get("lints")
if isinstance(lints, dict) and lints.get("workspace") is True:
    reject("workspace inheritance is forbidden for lints")
PYTHON
}

validate_detached_manifest() {
    local crate_dir=$1
    local metadata_file=$2
    local cargo_config

    for cargo_config in "$crate_dir/.cargo/config.toml" "$crate_dir/.cargo/config"; do
        if [[ -e "$cargo_config" || -L "$cargo_config" ]]; then
            printf 'batter-at-rest portability: crate-local Cargo configuration is forbidden: %s\n' \
                "$cargo_config" >&2
            return 1
        fi
    done

    if ! validate_source_manifest_policy "$crate_dir/Cargo.toml"; then
        return 1
    fi

    if ! cargo_msrv "$crate_dir" "$target_root/metadata" metadata \
        --no-deps \
        --format-version 1 >"$metadata_file"; then
        return 1
    fi

    if ! jq -e \
        --arg crate_dir "$crate_dir" \
        --arg package "$EXPECTED_PACKAGE" \
        --arg version "$EXPECTED_VERSION" \
        --arg license "$EXPECTED_LICENSE" \
        --arg rust "$EXPECTED_RUST_VERSION" '
            .workspace_root == $crate_dir
            and (.workspace_members | length == 1)
            and (.packages | length == 1)
            and .packages[0].name == $package
            and .packages[0].version == $version
            and .packages[0].edition == "2024"
            and .packages[0].rust_version == $rust
            and .packages[0].license == $license
            and .packages[0].publish == []
            and all(
                .packages[0].dependencies[];
                .source != null
                and (.source | startswith("registry+"))
                and .path == null
            )
        ' "$metadata_file" >/dev/null; then
        if jq -e '
            any(
                .packages[0].dependencies[];
                .source == null
                or ((.source | startswith("registry+")) | not)
                or .path != null
            )
        ' "$metadata_file" >/dev/null; then
            printf 'batter-at-rest portability: dependencies must use a registry source without a local path in %s\n' \
                "$crate_dir/Cargo.toml" >&2
            return 1
        fi
        printf 'batter-at-rest portability: detached metadata contract rejected for %s\n' \
            "$crate_dir/Cargo.toml" >&2
        jq -r \
            --arg crate_dir "$crate_dir" \
            --arg package "$EXPECTED_PACKAGE" \
            --arg version "$EXPECTED_VERSION" \
            --arg license "$EXPECTED_LICENSE" \
            --arg rust "$EXPECTED_RUST_VERSION" '
                [
                    if .workspace_root != $crate_dir then
                        "workspace_root: expected \($crate_dir | tojson), got \(.workspace_root | tojson)"
                    else empty end,
                    if (.workspace_members | length) != 1 then
                        "workspace_members: expected one, got \(.workspace_members | tojson)"
                    else empty end,
                    if (.packages | length) != 1 then
                        "packages: expected one, got \(.packages | length)"
                    else empty end,
                    if .packages[0].name != $package then
                        "package.name: expected \($package | tojson), got \(.packages[0].name | tojson)"
                    else empty end,
                    if .packages[0].version != $version then
                        "package.version: expected \($version | tojson), got \(.packages[0].version | tojson)"
                    else empty end,
                    if .packages[0].edition != "2024" then
                        "package.edition: expected \("2024" | tojson), got \(.packages[0].edition | tojson)"
                    else empty end,
                    if .packages[0].rust_version != $rust then
                        "package.rust_version: expected \($rust | tojson), got \(.packages[0].rust_version | tojson)"
                    else empty end,
                    if .packages[0].license != $license then
                        "package.license: expected \($license | tojson), got \(.packages[0].license | tojson)"
                    else empty end,
                    if .packages[0].publish != [] then
                        "package.publish: expected [], got \(.packages[0].publish | tojson)"
                    else empty end
                ] | .[]
            ' "$metadata_file" >&2
        return 1
    fi
}

expect_manifest_rejection() {
    local crate_dir=$1
    local metadata_file=$2
    local expected_message=$3
    local output_file=$4

    if validate_detached_manifest "$crate_dir" "$metadata_file" >"$output_file" 2>&1; then
        printf 'batter-at-rest portability: sensitivity check unexpectedly passed for %s\n' \
            "$crate_dir" >&2
        exit 1
    fi
    if ! grep -Fq "$expected_message" "$output_file"; then
        printf 'batter-at-rest portability: sensitivity check failed for the wrong reason: %s\n' \
            "$crate_dir" >&2
        while IFS= read -r output_line; do
            printf '%s\n' "$output_line" >&2
        done <"$output_file"
        exit 1
    fi
}

copy_crate() {
    local destination=$1
    local candidate_inventory="$portability_root/git-candidate-paths"
    local deleted_inventory="$portability_root/git-deleted-paths"
    local deleted_path
    local destination_dir
    local is_deleted
    local relative_path
    local repo_path
    local source_path
    local copied_files=0

    if ! git -C "$repo_root" ls-files -z \
        --cached \
        --others \
        --exclude-standard \
        -- crates/batter-at-rest >"$candidate_inventory"; then
        printf 'batter-at-rest portability: could not enumerate Git candidate paths\n' >&2
        exit 1
    fi
    if ! git -C "$repo_root" ls-files -z \
        --deleted \
        -- crates/batter-at-rest >"$deleted_inventory"; then
        printf 'batter-at-rest portability: could not enumerate deleted Git paths\n' >&2
        exit 1
    fi

    mkdir -p "$destination"
    while IFS= read -r -d '' repo_path; do
        is_deleted=false
        while IFS= read -r -d '' deleted_path; do
            if [[ "$repo_path" == "$deleted_path" ]]; then
                is_deleted=true
                break
            fi
        done <"$deleted_inventory"
        if [[ "$is_deleted" == true ]]; then
            continue
        fi

        relative_path=${repo_path#crates/batter-at-rest/}
        if [[ -z "$relative_path" || "$relative_path" == "$repo_path" ]]; then
            printf 'batter-at-rest portability: invalid crate candidate path: %s\n' "$repo_path" >&2
            exit 1
        fi
        source_path="$repo_root/$repo_path"
        if [[ -L "$source_path" ]]; then
            printf 'batter-at-rest portability: Git candidate must not contain symlinks: %s\n' \
                "$repo_path" >&2
            exit 1
        fi
        if [[ ! -f "$source_path" ]]; then
            printf 'batter-at-rest portability: Git candidate path is missing or not a regular file: %s\n' \
                "$repo_path" >&2
            exit 1
        fi
        destination_dir=$(dirname "$destination/$relative_path")
        mkdir -p "$destination_dir"
        cp -p -- "$source_path" "$destination/$relative_path"
        copied_files=$((copied_files + 1))
    done <"$candidate_inventory"
    if [[ $copied_files -eq 0 || ! -f "$destination/Cargo.toml" ]]; then
        printf 'batter-at-rest portability: Git candidate inventory omitted the crate manifest\n' >&2
        exit 1
    fi
}

run_sensitivity_checks() {
    local sensitivity_root="$portability_root/sensitivity"
    local workspace_crate="$sensitivity_root/workspace/batter-at-rest"
    local path_root="$sensitivity_root/path"
    local path_crate="$path_root/bad-path"
    local path_dependency="$path_root/outside"
    local patch_crate="$path_root/bad-patch"
    local config_root="$sensitivity_root/config"
    local config_crate="$config_root/batter-at-rest"
    local ambient_cargo_home="$config_root/ambient-cargo-home"
    local location_parent="$sensitivity_root/location/parent"
    local location_child="$location_parent/child"

    mkdir -p "$location_child"
    printf '%s\n' '[workspace]' >"$location_parent/Cargo.toml"
    if assert_no_parent_cargo_manifest \
        "$location_child" >"$sensitivity_root/location-control.txt" 2>&1; then
        printf 'batter-at-rest portability: checkout-location sensitivity control unexpectedly passed\n' >&2
        exit 1
    fi
    if ! grep -Fq \
        "Cargo manifest exists on isolated launch path: $location_parent/Cargo.toml" \
        "$sensitivity_root/location-control.txt"; then
        printf 'batter-at-rest portability: checkout-location sensitivity control failed for the wrong reason\n' >&2
        exit 1
    fi

    if (
        export GIT_DIR="$sensitivity_root/missing.git"
        copy_crate "$sensitivity_root/git-failure"
    ) >"$sensitivity_root/git-failure-control.txt" 2>&1; then
        printf 'batter-at-rest portability: Git-inventory sensitivity control unexpectedly passed\n' >&2
        exit 1
    fi
    if ! grep -Fq 'could not enumerate Git candidate paths' \
        "$sensitivity_root/git-failure-control.txt"; then
        printf 'batter-at-rest portability: Git-inventory sensitivity control failed for the wrong reason\n' >&2
        exit 1
    fi

    copy_crate "$workspace_crate"
    awk '
        /^\[package\]$/ { in_package = 1; print; next }
        /^\[/ { in_package = 0 }
        in_package && /^version[[:space:]]*=/ { print "version.\"workspace\" = true"; next }
        { print }
    ' "$workspace_crate/Cargo.toml" >"$workspace_crate/Cargo.toml.mutated"
    mv "$workspace_crate/Cargo.toml.mutated" "$workspace_crate/Cargo.toml"
    printf '%s\n' \
        '' \
        '[workspace]' \
        'resolver = "2"' \
        '' \
        '[workspace.package]' \
        'version = "0.1.0"' >>"$workspace_crate/Cargo.toml"
    if ! grep -Fqx 'version."workspace" = true' \
        "$workspace_crate/Cargo.toml"; then
        printf 'batter-at-rest portability: workspace sensitivity mutation did not apply\n' >&2
        exit 1
    fi
    expect_manifest_rejection \
        "$workspace_crate" \
        "$sensitivity_root/workspace-metadata.json" \
        'workspace inheritance is forbidden' \
        "$sensitivity_root/workspace-rejection.txt"

    copy_crate "$path_crate"
    mkdir -p "$path_dependency/src"
    printf '%s\n' \
        '[package]' \
        'name = "portability-probe"' \
        'version = "0.0.0"' \
        'edition = "2021"' >"$path_dependency/Cargo.toml"
    printf '%s\n' '#![forbid(unsafe_code)]' >"$path_dependency/src/lib.rs"
    awk '
        { print }
        /^\[dependencies\]$/ {
            print "portability-probe = { path = \"../outside\" }"
        }
    ' "$path_crate/Cargo.toml" >"$path_crate/Cargo.toml.mutated"
    mv "$path_crate/Cargo.toml.mutated" "$path_crate/Cargo.toml"
    if ! grep -Fqx 'portability-probe = { path = "../outside" }' \
        "$path_crate/Cargo.toml"; then
        printf 'batter-at-rest portability: path sensitivity mutation did not apply\n' >&2
        exit 1
    fi
    expect_manifest_rejection \
        "$path_crate" \
        "$path_root/metadata.json" \
        'dependencies must use a registry source without a local path' \
        "$path_root/rejection.txt"

    copy_crate "$patch_crate"
    printf '%s\n' \
        '' \
        '[patch]' \
        'crates-io.zeroize = { path = "../patched-zeroize" }' >>"$patch_crate/Cargo.toml"
    expect_manifest_rejection \
        "$patch_crate" \
        "$path_root/patch-metadata.json" \
        'dependency source override tables are forbidden' \
        "$path_root/patch-rejection.txt"

    copy_crate "$config_crate"
    printf '%s\n' \
        '' \
        '# workspace = true and [patch.crates-io] are inert inside comments.' \
        >>"$config_crate/Cargo.toml"
    mkdir -p "$config_root/.cargo" "$ambient_cargo_home"
    printf '%s\n' 'this is intentionally invalid Cargo configuration' \
        >"$config_root/.cargo/config.toml"
    printf '%s\n' 'this is intentionally invalid Cargo configuration' \
        >"$ambient_cargo_home/config.toml"
    if (
        cd "$config_crate"
        "${cargo_environment[@]}" \
            "CARGO_TARGET_DIR=$config_root/control-target" \
            cargo "+$REQUIRED_RUST_TOOLCHAIN" metadata \
            --manifest-path "$config_crate/Cargo.toml" \
            --no-deps \
            --format-version 1
    ) >"$config_root/control-output.txt" 2>&1; then
        printf 'batter-at-rest portability: Cargo-config sensitivity control unexpectedly passed\n' >&2
        exit 1
    fi
    if ! grep -Eq 'could not (load|parse).*Cargo configuration' \
        "$config_root/control-output.txt"; then
        printf 'batter-at-rest portability: Cargo-config sensitivity control failed for the wrong reason\n' >&2
        while IFS= read -r output_line; do
            printf '%s\n' "$output_line" >&2
        done <"$config_root/control-output.txt"
        exit 1
    fi
    (
        export CARGO_HOME="$ambient_cargo_home"
        export CARGO_TARGET_DIR="$config_root/ambient-target"
        export RUSTFLAGS="--intentionally-invalid-portability-flag"
        validate_detached_manifest "$config_crate" "$config_root/metadata.json"
        cargo_msrv "$config_crate" "$target_root/config" check --lib
    )

    mkdir -p "$config_crate/.cargo"
    cp "$config_root/.cargo/config.toml" "$config_crate/.cargo/config.toml"
    expect_manifest_rejection \
        "$config_crate" \
        "$config_root/local-config-metadata.json" \
        'crate-local Cargo configuration is forbidden' \
        "$config_root/local-config-rejection.txt"

    printf 'batter-at-rest portability: Git inventory, workspace, dependency-source, parent-path, and Cargo-config sensitivity checks passed\n'
}

run_sensitivity_checks

detached_crate="$portability_root/detached/batter-at-rest"
copy_crate "$detached_crate"
validate_detached_manifest "$detached_crate" "$portability_root/detached-metadata.json"
node "$detached_crate/tests/fixtures/generate-envelope-v1.mjs" --check
printf 'batter-at-rest portability: detached manifest and dependency metadata passed\n'

cargo_msrv "$detached_crate" "$target_root/detached" generate-lockfile
cargo_msrv "$detached_crate" "$target_root/detached" test --locked
cargo_msrv "$detached_crate" "$target_root/detached" test --test public_api --locked
cargo_msrv "$detached_crate" "$target_root/detached" check --all-targets --locked
printf 'batter-at-rest portability: detached source tests and all-target check passed\n'

cargo_msrv "$detached_crate" "$target_root/detached" package --allow-dirty
package_files=("$target_root"/detached/package/batter-at-rest-*.crate)
if [[ ${#package_files[@]} -ne 1 || ! -f "${package_files[0]}" ]]; then
    printf 'batter-at-rest portability: expected exactly one packaged crate artifact\n' >&2
    exit 1
fi

unpacked_root="$portability_root/unpacked"
mkdir -p "$unpacked_root"
tar -xzf "${package_files[0]}" -C "$unpacked_root"
unpacked_dirs=("$unpacked_root"/batter-at-rest-*)
if [[ ${#unpacked_dirs[@]} -ne 1 || ! -d "${unpacked_dirs[0]}" ]]; then
    printf 'batter-at-rest portability: expected exactly one unpacked crate directory\n' >&2
    exit 1
fi
unpacked_crate=${unpacked_dirs[0]}

for required_artifact_path in \
    AGENTS.md \
    Cargo.lock \
    LICENSE \
    README.md \
    docs/format-v1.md \
    tests/fixtures/generate-envelope-v1.mjs \
    tests/fixtures/envelope-v1.txt \
    tests/public_api.rs; do
    if [[ ! -f "$unpacked_crate/$required_artifact_path" ]]; then
        printf 'batter-at-rest portability: packaged artifact omitted required path: %s\n' \
            "$required_artifact_path" >&2
        exit 1
    fi
done
validate_detached_manifest "$unpacked_crate" "$portability_root/package-metadata.json"
cargo_msrv "$unpacked_crate" "$target_root/unpacked" test --locked
cargo_msrv "$unpacked_crate" "$target_root/unpacked" test --test public_api --locked
cargo_msrv "$unpacked_crate" "$target_root/unpacked" check --all-targets --locked
printf 'batter-at-rest portability: verified package and unpacked artifact tests passed\n'
printf 'batter-at-rest portability: public consumer test passed in source and packaged artifacts\n'
