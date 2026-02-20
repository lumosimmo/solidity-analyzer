#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: ./bump-versions.sh <semver>

Examples:
  ./bump-versions.sh 0.1.2
  ./bump-versions.sh v0.1.2-rc.1

This script:
1) updates all workspace crate versions and the workspace package version
2) updates editors/code/package.json version
3) runs cargo check and bun install to refresh lock files
4) creates a release commit when there are version changes
5) creates a local annotated git tag v<semver>
EOF
}

if [[ $# -ne 1 ]]; then
    usage >&2
    exit 1
fi

input_version="$1"
version="${input_version#v}"

semver_regex='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?(\+([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$'
if [[ ! "${version}" =~ ${semver_regex} ]]; then
    echo "Invalid semver: '${input_version}'" >&2
    usage >&2
    exit 1
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "${script_dir}"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "This script must be run inside a git repository." >&2
    exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
    echo "Working tree is not clean. Commit/stash changes before running this script." >&2
    exit 1
fi

tag="v${version}"
if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null; then
    echo "Tag ${tag} already exists." >&2
    exit 1
fi

update_toml_section_version() {
    local file="$1"
    local section="$2"
    local tmp_file

    tmp_file="$(mktemp)"
    if ! awk -v section="${section}" -v version="${version}" '
        BEGIN {
            in_section = 0
            updated = 0
        }
        /^\[.*\]$/ {
            in_section = ($0 == "[" section "]")
        }
        {
            if (in_section && $0 ~ /^[[:space:]]*version[[:space:]]*=/) {
                print "version = \"" version "\""
                updated = 1
            } else {
                print $0
            }
        }
        END {
            if (!updated) {
                exit 2
            }
        }
    ' "${file}" >"${tmp_file}"; then
        rm -f "${tmp_file}"
        echo "Failed to update version in ${file} section [${section}]." >&2
        exit 1
    fi

    mv "${tmp_file}" "${file}"
}

update_extension_version() {
    local file="editors/code/package.json"
    local tmp_file

    tmp_file="$(mktemp)"
    if ! awk -v version="${version}" '
        BEGIN { updated = 0 }
        {
            if (!updated && $0 ~ /^[[:space:]]*"version"[[:space:]]*:/) {
                sub(/"version"[[:space:]]*:[[:space:]]*"[^"]+"/, "\"version\": \"" version "\"")
                updated = 1
            }
            print $0
        }
        END {
            if (!updated) {
                exit 2
            }
        }
    ' "${file}" >"${tmp_file}"; then
        rm -f "${tmp_file}"
        echo "Failed to update extension version in ${file}." >&2
        exit 1
    fi

    mv "${tmp_file}" "${file}"
}

update_toml_section_version "Cargo.toml" "workspace.package"

mapfile -t cargo_files < <(find crates xtask -type f -name Cargo.toml | sort)
if [[ ${#cargo_files[@]} -eq 0 ]]; then
    echo "No crate Cargo.toml files found." >&2
    exit 1
fi

for cargo_file in "${cargo_files[@]}"; do
    update_toml_section_version "${cargo_file}" "package"
done

update_extension_version

echo "Running cargo check (may update Cargo.lock)..."
cargo check

echo "Running bun install in editors/code (may update bun.lock)..."
(
    cd editors/code
    bun install
)

git add Cargo.toml Cargo.lock editors/code/package.json editors/code/bun.lock "${cargo_files[@]}"

if git diff --cached --quiet; then
    echo "No version changes were needed. Creating tag ${tag} on current HEAD."
else
    git commit -m "chore(release): bump version to ${version}"
fi

git tag -a "${tag}" -m "${tag}"

echo "Done."
echo "Version: ${version}"
echo "Tag: ${tag}"
echo "Commit: $(git rev-parse --short HEAD)"
