#!/usr/bin/env nu
#
# Debug probe for the cargo-build-sbf wrapper.
#
# Drives the dev-shell `cargo-build-sbf` with verbose tracing enabled and
# captures every invocation of the rustc shim. Use it to figure out why
# solana-cli's cargo-build-sbf is rejecting the toolchain, or to see what
# rustc commands cargo-build-sbf is issuing under the hood.
#
# Outputs (relative to repo root):
#   .tmp/rustc-shim.log         every spawn of the rustc shim, with argv
#   .tmp/cargo-build-sbf.log    full RUST_LOG=trace stderr/stdout from cargo-build-sbf
#
# Usage (from inside the dev shell):
#   nu scripts/probe-cargo-build-sbf.nu --manifest-path programs/fund/Cargo.toml
#
# All `...args` are forwarded verbatim to `cargo-build-sbf build-sbf`.

# Pure helper, exported so the test suite can exercise it.
# Pulls the chronological list of `rustc-shim: <argv>` entries out of a shim
# log produced by the rustc shim. Returns each entry's argv as a list.
export def parse-shim-log [log: string]: nothing -> list<list<string>> {
  $log
    | lines
    | where ($it | str starts-with "rustc-shim: ")
    | each {|line| $line | str substring 12.. | split row " " }
}

def --wrapped main [
  --clean         # delete .devenv/sbf-home/ first so the wrapper repopulates from scratch
  --sbf-home: string = ".devenv/sbf-home"   # where the wrapper materializes platform-tools
  ...args: string
] {
  let tmp = (pwd | path join ".tmp")
  mkdir $tmp
  let shim_log = ($tmp | path join "rustc-shim.log")
  let sbf_log = ($tmp | path join "cargo-build-sbf.log")
  for f in [$shim_log $sbf_log] {
    if ($f | path exists) { rm $f }
  }
  if $clean and ($sbf_home | path exists) {
    rm -rf $sbf_home
  }
  let result = (
    with-env { RUSTC_SHIM_LOG: $shim_log, RUST_LOG: "trace" } {
      do { ^cargo-build-sbf build-sbf ...$args } | complete
    }
  )
  $"($result.stdout)\n--- stderr ---\n($result.stderr)" | save --force --raw $sbf_log
  print $"# cargo-build-sbf exit code: ($result.exit_code)"
  print $"# Shim spawns \(($shim_log)\):"
  if ($shim_log | path exists) {
    let entries = parse-shim-log (open --raw $shim_log)
    if (($entries | length) == 0) {
      print "  <shim was not invoked>"
    } else {
      $entries | each {|e| print $"  rustc ($e | str join ' ')" }
    }
  } else {
    print "  <no log produced — cargo-build-sbf never called the shim>"
  }
  print ""
  print $"# cargo-build-sbf trace tail \(($sbf_log)\):"
  let log_lines = (open --raw $sbf_log | lines)
  let tail_lines = if (($log_lines | length) > 30) {
    $log_lines | last 30
  } else {
    $log_lines
  }
  for l in $tail_lines { print $"  ($l)" }
}
