#!/usr/bin/env nu
#
# Project-local wrapper for `cargo-build-sbf` (from `solana-cli`).
#
# Behavior:
#   1. Point `HOME` at `.devenv/sbf-home/` inside the repo so cargo-build-sbf
#      downloads platform-tools and keeps its state under `.devenv/` instead
#      of polluting the user's home cache. First `anchor build` is online
#      (cargo-build-sbf fetches platform-tools from
#      `github.com/anza-xyz/platform-tools/releases`); subsequent builds reuse
#      the local cache and run offline.
#   2. Strip the leading `build-sbf` subcommand that `anchor build` injects
#      (because cargo-build-sbf is invoked as a cargo extension).
#   3. Re-exec the real binary with `--no-rustup-override` (rustup is not in
#      the dev shell).
#
# Required environment (set by the nix wrapper in flake.nix):
#   CARGO_BUILD_SBF_REAL_BIN  absolute path to solana-cli's cargo-build-sbf
#   CARGO_BUILD_SBF_HOME      absolute path used as HOME for cargo-build-sbf

# Pure helper, exported so the test suite can exercise it.
export def strip-build-sbf [args: list<string>]: nothing -> list<string> {
  if (($args | length) > 0) and (($args | first) == "build-sbf") {
    $args | skip 1
  } else {
    $args
  }
}

def --wrapped main [...args: string] {
  let real_bin = ($env.CARGO_BUILD_SBF_REAL_BIN? | default "")
  let sbf_home = ($env.CARGO_BUILD_SBF_HOME? | default "")
  if ($real_bin == "") or ($sbf_home == "") {
    error make { msg: "CARGO_BUILD_SBF_REAL_BIN and CARGO_BUILD_SBF_HOME must be set" }
  }
  mkdir $sbf_home
  let forwarded = strip-build-sbf $args
  with-env { HOME: $sbf_home } {
    ^$real_bin --no-rustup-override ...$forwarded
  }
}
