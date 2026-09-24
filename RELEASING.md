# Releasing gateflow

## Crates

The root crate depends on `gateflow-macros` by version when it is published. Publish the companion crate first, wait for it to become available from crates.io, then publish the root crate:

```sh
cargo publish -p gateflow-macros
cargo publish -p gateflow
```

Run the release gates before publishing. The package dry-run for `gateflow` will fail until the matching `gateflow-macros` version is visible in the registry; this is expected Cargo behavior for the path-plus-version dependency.

## Signed CLI binaries

After the crates are ready, create and push a semver tag from a clean commit:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds Linux x86_64 and aarch64 archives, creates `SHA256SUMS`, signs that manifest with keyless Sigstore signing through GitHub Actions OIDC, and publishes the archives plus the signature bundle to a GitHub Release.

The installer intentionally requires `cosign` and refuses to install until both the signed manifest and the selected archive verify:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/cybercore-tech/gateflow/raw/v0.1.0/scripts/install.sh | sh
```

Do not point the installer at a branch URL. Keep user-facing install commands pinned to a release tag.
