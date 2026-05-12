#!/usr/bin/env nu
#
# Test suite for `cargo-build-sbf.nu`. Runs as the `checkPhase` of the
# wrapper's nix derivation — if any assertion fails the script exits
# non-zero and the build fails.

use std/assert
use ./cargo-build-sbf.nu *

def test_strip_build_sbf_drops_leading_subcommand [] {
  let result = (strip-build-sbf ["build-sbf" "--release" "--features" "foo"])
  assert equal $result ["--release" "--features" "foo"]
}

def test_strip_build_sbf_passes_through_when_absent [] {
  let result = (strip-build-sbf ["--release" "build-sbf"])
  assert equal $result ["--release" "build-sbf"]
}

def test_strip_build_sbf_handles_empty [] {
  let result = (strip-build-sbf [])
  assert equal $result []
}

def test_strip_build_sbf_only_strips_first_position [] {
  let result = (strip-build-sbf ["build-sbf" "build-sbf"])
  assert equal $result ["build-sbf"]
}

def main [] {
  test_strip_build_sbf_drops_leading_subcommand
  test_strip_build_sbf_passes_through_when_absent
  test_strip_build_sbf_handles_empty
  test_strip_build_sbf_only_strips_first_position
  print "cargo-build-sbf.test.nu: all tests passed"
}
