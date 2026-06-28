#!/usr/bin/env nu
#
# Test suite for `cargo-rustup-arg-shim.nu`. Runs as part of the sbfScripts nix
# derivation's checkPhase — a failing assertion exits non-zero and fails the
# build.

use std/assert
use ./cargo-rustup-arg-shim.nu *

def test_strips_leading_toolchain_selector [] {
  assert equal (strip-toolchain ["+1.95.0" "test" "--lib"]) ["test" "--lib"]
}

def test_preserves_non_selector_first_arg [] {
  assert equal (strip-toolchain ["test" "--lib"]) ["test" "--lib"]
}

def test_handles_zero_args [] {
  assert equal (strip-toolchain []) []
}

def test_strips_only_the_first_position [] {
  assert equal (strip-toolchain ["+stable" "+nightly"]) ["+nightly"]
}

def test_selector_only [] {
  assert equal (strip-toolchain ["+1.95.0"]) []
}

def main [] {
  test_strips_leading_toolchain_selector
  test_preserves_non_selector_first_arg
  test_handles_zero_args
  test_strips_only_the_first_position
  test_selector_only
  print "cargo-rustup-arg-shim.test.nu: all tests passed"
}
