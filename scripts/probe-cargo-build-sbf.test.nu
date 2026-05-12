#!/usr/bin/env nu
#
# Test suite for `probe-cargo-build-sbf.nu`. Runs in the script derivation's
# `checkPhase`. Covers the pure parsing helper — the side-effecting `main`
# entry point is not exercised here because it needs a real cargo-build-sbf
# binary to drive.

use std/assert
use ./probe-cargo-build-sbf.nu *

def test_parse_shim_log_empty [] {
  let result = (parse-shim-log "")
  assert equal $result []
}

def test_parse_shim_log_extracts_argv [] {
  let log = "rustc-shim: --version
rustc-shim: --print target-list
unrelated noise
rustc-shim: --target sbpf-solana-solana --crate-name fund"
  let result = (parse-shim-log $log)
  assert equal $result [
    ["--version"]
    ["--print" "target-list"]
    ["--target" "sbpf-solana-solana" "--crate-name" "fund"]
  ]
}

def test_parse_shim_log_skips_non_shim_lines [] {
  let log = "warning: random output
rustc-shim: --version
INFO some log line"
  let result = (parse-shim-log $log)
  assert equal $result [["--version"]]
}

def main [] {
  test_parse_shim_log_empty
  test_parse_shim_log_extracts_argv
  test_parse_shim_log_skips_non_shim_lines
  print "probe-cargo-build-sbf.test.nu: all tests passed"
}
