#!/usr/bin/env nu
#
# Regenerate `Cargo.lock` using the platform-tools cargo (1.84) instead of
# the host nixpkgs cargo (1.95). The host cargo's `cargo update` may resolve
# to dependency versions that require `edition2024`, which platform-tools
# cargo 1.84 cannot parse. Re-resolving under the older cargo nudges the
# resolver toward older, edition2021-compatible dependency versions.
#
# Side effects:
#   - Deletes `Cargo.lock`.
#   - Spawns `cargo generate-lockfile` via the cargo-build-sbf wrapper's
#     PATH (which prepends platform-tools/rust/bin), so the cargo binary
#     resolves to platform-tools cargo.

def main [
  --manifest-path: string = "programs/fund/Cargo.toml"
  --sbf-home: string = ".devenv/sbf-home"
  --tools-version: string = "1.51"
] {
  let cache_dir = ($sbf_home | path join ".cache" "solana" $"v($tools_version)" "platform-tools")
  let bin_dir = ($cache_dir | path join "rust" "bin")
  if not ($bin_dir | path exists) {
    error make { msg: $"platform-tools not populated yet at ($bin_dir) — run the cargo-build-sbf wrapper or `cargo-build-sbf build-sbf` once first" }
  }
  if ("Cargo.lock" | path exists) {
    print "removing existing Cargo.lock"
    rm Cargo.lock
  }
  print "regenerating Cargo.lock with platform-tools cargo"
  with-env { HOME: $sbf_home, PATH: $"($bin_dir):($env.PATH)" } {
    ^cargo generate-lockfile --manifest-path $manifest_path
  }
  print "done"
}
