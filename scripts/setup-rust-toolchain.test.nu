#!/usr/bin/env nu
#
# Test that setup-rust-toolchain.nu materializes the expected layout
# (rustup-home/proxies/{cargo,rustc,…} symlinks, rustup-home/toolchains
# directory) and emits parseable `export` lines.

use std/assert

# `path self` is a parse-time constant that resolves to the absolute
# path of this file, regardless of where it's invoked from. We use it
# to find the sibling `setup-rust-toolchain.nu` so the test works both
# locally (cwd = repo root) and inside the nix `checkPhase` (cwd = the
# source root which is the `scripts/` directory).
const SETUP_SCRIPT = (path self setup-rust-toolchain.nu)

# Build the invocation as a closure so we can call it twice in the
# idempotency test without repeating the long line.
def run-setup [
  tmp: string
  fake_rustup: string
  fake_toolchain: string
]: nothing -> record {
  do { ^nu $SETUP_SCRIPT "--devenv-root" $tmp "--rustup-bin" $fake_rustup "--rust-toolchain" $fake_toolchain } | complete
}

# Set up a tempdir with a fake rustup binary that always exits 0. Lets
# the test run without the real rustup on PATH.
def make-fixtures []: nothing -> record {
  let tmp = (mktemp -d)
  let fake_rustup = ($tmp | path join "rustup")
  "#!/bin/sh
exit 0" | save --force $fake_rustup
  chmod +x $fake_rustup
  let fake_toolchain = ($tmp | path join "toolchain-root")
  mkdir $fake_toolchain
  { tmp: $tmp, rustup: $fake_rustup, toolchain: $fake_toolchain }
}

def test_setup_creates_proxies_and_exports [] {
  let fx = (make-fixtures)
  let result = (run-setup $fx.tmp $fx.rustup $fx.toolchain)
  assert equal $result.exit_code 0

  let rustup_home = ($fx.tmp | path join ".devenv" "rustup-home")
  let proxy_bin = ($rustup_home | path join "proxies")
  for tool in ["cargo" "rustc" "rustfmt" "clippy"] {
    let p = ($proxy_bin | path join $tool)
    assert ($p | path exists)
    assert equal (readlink $p | str trim) $fx.rustup
  }
  assert ($rustup_home | path join "toolchains" | path exists)

  assert ($result.stdout | str contains "export RUSTUP_HOME=")
  assert ($result.stdout | str contains "export RUSTUP_TOOLCHAIN=nix")
  assert ($result.stdout | str contains "export PATH=")

  rm -rf $fx.tmp
}

def test_setup_is_idempotent [] {
  let fx = (make-fixtures)
  let first = (run-setup $fx.tmp $fx.rustup $fx.toolchain)
  assert equal $first.exit_code 0
  let second = (run-setup $fx.tmp $fx.rustup $fx.toolchain)
  assert equal $second.exit_code 0
  rm -rf $fx.tmp
}

def main [] {
  test_setup_creates_proxies_and_exports
  test_setup_is_idempotent
  print "setup-rust-toolchain.test.nu: all tests passed"
}
