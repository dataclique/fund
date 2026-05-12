#!/usr/bin/env nu
#
# Quick probe: invoke `cargo metadata` and `rustc -vV` directly with the
# rustc shim on PATH, so we can see whether cargo agrees with what the
# shim produces. Useful when cargo-build-sbf complains that `rustc -vV`
# is missing the `host:` line — this script tells you whether the shim
# is the culprit or whether cargo is sourcing rustc from somewhere else.

def main [
  --sbf-home: string = ".devenv/sbf-home"
  --tools-version: string = "1.51"
] {
  let shim_bin = ($sbf_home | path join ".cache" "solana" $"v($tools_version)" "platform-tools" "rust" "bin")
  if not ($shim_bin | path exists) {
    error make { msg: $"shim bin dir not found: ($shim_bin) — run the cargo-build-sbf wrapper at least once first" }
  }

  for probe in [["-vV"] ["--version"] ["--print" "target-list"]] {
    print $"# Direct invocation: rustc ($probe | str join ' ')"
    with-env { PATH: $"($shim_bin):($env.PATH)" } {
      let r = (do { ^rustc ...$probe } | complete)
      print $"  exit: ($r.exit_code)"
      print "  stdout (first 12 lines):"
      $r.stdout | lines | first 12 | each {|l| print $"    ($l)" }
      if ($r.stderr | str length) > 0 {
        print "  stderr:"
        $r.stderr | lines | each {|l| print $"    ($l)" }
      }
    }
    print ""
  }
  print "# Does target-list contain sbpf-solana-solana?"
  with-env { PATH: $"($shim_bin):($env.PATH)" } {
    let targets = (do { ^rustc --print target-list } | complete)
    let found = ($targets.stdout | lines | any {|l| $l == "sbpf-solana-solana" })
    print $"  ($found)"
  }

  print ""
  print "# cargo metadata (via rustc-only-bin shim PATH)"
  let rustc_only = ($sbf_home | path join "rustc-only-bin")
  with-env { PATH: $"($rustc_only):($env.PATH)" } {
    let r = (do { ^cargo metadata --no-deps --format-version 1 --manifest-path programs/fund/Cargo.toml } | complete)
    print $"  exit: ($r.exit_code)"
    if ($r.stderr | str length) > 0 {
      print "  stderr:"
      $r.stderr | lines | each {|l| print $"    ($l)" }
    }
    if $r.exit_code == 0 {
      print "  stdout (first 200 chars):"
      print $"    ($r.stdout | str substring 0..200)..."
    }
  }

  print ""
  print "# which cargo is on PATH (rustc-only mode)?"
  with-env { PATH: $"($rustc_only):($env.PATH)" } {
    let r = (do { ^which cargo } | complete)
    print $"  ($r.stdout | str trim)"
  }
}
