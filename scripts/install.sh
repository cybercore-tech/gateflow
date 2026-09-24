#!/bin/sh
set -eu

die() {
    echo "gateflow installer: $*" >&2
    exit 1
}

repo="${GATEFLOW_REPO:-cybercore-tech/gateflow}"
requested_version="${GATEFLOW_VERSION:-latest}"
home="${HOME:-}"
install_dir="${GATEFLOW_INSTALL_DIR:-}"
if [ -z "$install_dir" ]; then
    [ -n "$home" ] || die "HOME is not set; provide GATEFLOW_INSTALL_DIR"
    install_dir="${XDG_BIN_HOME:-$home/.local/bin}"
fi
api_url="https://api.github.com/repos/$repo/releases/latest"

need_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

need_command curl
need_command tar
need_command sha256sum
need_command cosign
need_command install

case "$(uname -s)" in
    Linux) ;;
    *) die "gateflow binary releases currently support Linux only" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
    *) die "unsupported Linux architecture: $(uname -m)" ;;
esac

if [ "$requested_version" = "latest" ]; then
    release_tag="$(curl --proto '=https' --tlsv1.2 -fsSL "$api_url" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1)"
    [ -n "$release_tag" ] || die "could not determine the latest release tag"
else
    release_tag="$requested_version"
    case "$release_tag" in
        v*) ;;
        *) release_tag="v$release_tag" ;;
    esac
fi

version="${release_tag#v}"
asset="gateflow-${version}-${target}.tar.gz"
base_url="https://github.com/$repo/releases/download/$release_tag"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

echo "Downloading gateflow $release_tag for $target..."
curl --proto '=https' --tlsv1.2 -fsSL "$base_url/$asset" -o "$tmp_dir/$asset"
curl --proto '=https' --tlsv1.2 -fsSL "$base_url/SHA256SUMS" -o "$tmp_dir/SHA256SUMS"
curl --proto '=https' --tlsv1.2 -fsSL "$base_url/SHA256SUMS.bundle" -o "$tmp_dir/SHA256SUMS.bundle"

cosign verify-blob \
    --bundle "$tmp_dir/SHA256SUMS.bundle" \
    --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$release_tag$" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
    "$tmp_dir/SHA256SUMS" >/dev/null \
    || die "signed checksum manifest verification failed"

expected="$(awk -v asset="$asset" '$2 == asset { print $1 }' "$tmp_dir/SHA256SUMS")"
[ -n "$expected" ] || die "checksum manifest does not contain $asset"
actual="$(sha256sum "$tmp_dir/$asset" | awk '{ print $1 }')"
[ "$expected" = "$actual" ] || die "binary checksum mismatch"

mkdir -p "$install_dir"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
install -m 0755 "$tmp_dir/gateflow-${version}-${target}/gateflow" "$install_dir/gateflow"

echo "Installed gateflow $release_tag to $install_dir/gateflow"
case ":${PATH:-}:" in
    *":$install_dir:"*) ;;
    *) echo "Add $install_dir to PATH if it is not already present." ;;
esac
