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

# ensure-platform-tools is exercised end-to-end against a real filesystem.
def test_ensure_populates_when_missing [] {
  let tmp = (mktemp -d)
  let source = ($tmp | path join "source")
  mkdir ($source | path join "rust" "bin")
  "fake-rustc" | save ($source | path join "rust" "bin" "rustc")
  let cache = ($tmp | path join "cache" "platform-tools")

  ensure-platform-tools $source $cache

  assert ($cache | path exists)
  assert ($cache | path join "rust" "bin" "rustc" | path exists)
  let marker = (open --raw ($cache | path join ".source") | str trim)
  assert equal $marker $source

  rm -rf $tmp
}

def test_ensure_no_op_when_marker_matches [] {
  let tmp = (mktemp -d)
  let source = ($tmp | path join "source")
  mkdir $source
  "v1" | save ($source | path join "marker")
  let cache = ($tmp | path join "cache")

  ensure-platform-tools $source $cache
  "tampered" | save --force ($cache | path join "marker")
  ensure-platform-tools $source $cache

  let after = (open --raw ($cache | path join "marker") | str trim)
  assert equal $after "tampered"

  rm -rf $tmp
}

def test_ensure_repopulates_on_source_change [] {
  let tmp = (mktemp -d)
  let source_a = ($tmp | path join "a")
  let source_b = ($tmp | path join "b")
  mkdir $source_a
  mkdir $source_b
  "from-a" | save ($source_a | path join "marker")
  "from-b" | save ($source_b | path join "marker")
  let cache = ($tmp | path join "cache")

  ensure-platform-tools $source_a $cache
  let first = (open --raw ($cache | path join "marker") | str trim)
  assert equal $first "from-a"

  ensure-platform-tools $source_b $cache
  let second = (open --raw ($cache | path join "marker") | str trim)
  assert equal $second "from-b"

  rm -rf $tmp
}

# install-rustc-shim: the shim must emit the toolchain marker for --version
# and forward every other invocation verbatim to the real rustc.
def test_shim_adds_toolchain_marker_to_version [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  "#!/bin/sh
echo 'rustc 1.89.0-dev'" | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"

  let out = (^$real --version | str trim)
  assert equal $out "rustc 1.89.0-dev toolchain-v1.52"
  assert ($bin | path join "rustc.real" | path exists)

  rm -rf $tmp
}

# cargo-build-sbf uses `-vV` (short verbose-version) in addition to
# `--version`; the shim has to recognize all the common spellings.
def test_shim_adds_marker_for_short_verbose_version [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  "#!/bin/sh
echo 'rustc 1.89.0-dev'
echo 'host: aarch64-apple-darwin'
echo 'release: 1.89.0-dev'" | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"

  let lines = (^$real -vV | lines)
  assert equal ($lines | first) "rustc 1.89.0-dev toolchain-v1.52"
  assert equal ($lines | get 1) "host: aarch64-apple-darwin"
  assert equal ($lines | get 2) "release: 1.89.0-dev"

  rm -rf $tmp
}

def test_shim_adds_marker_for_separate_verbose_flag [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  "#!/bin/sh
echo 'rustc 1.89.0-dev'" | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"

  let out = (^$real --version --verbose | str trim)
  assert equal $out "rustc 1.89.0-dev toolchain-v1.52"

  rm -rf $tmp
}

def test_shim_forwards_other_args [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  let real_body = "#!/bin/sh
echo \"called-with: $*\""
  $real_body | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"

  let out = (^$real --print target-list --target sbpf-solana-solana | str trim)
  assert equal $out "called-with: --print target-list --target sbpf-solana-solana"

  rm -rf $tmp
}

# Regression test: cargo/cargo-build-sbf invoke rustc with a stripped PATH
# (no `head`, `tail`, `awk`, `cat`, etc.). The shim must work with shell
# builtins only.
def test_shim_works_with_stripped_path [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  "#!/bin/sh
echo 'rustc 1.89.0-dev'
echo 'host: aarch64-apple-darwin'" | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"

  # Stripped PATH: only the shim's own dir. No coreutils available.
  let out = (with-env { PATH: $bin } { ^$real -vV } | str trim)
  let lines = ($out | lines)
  assert equal ($lines | first) "rustc 1.89.0-dev toolchain-v1.52"
  assert equal ($lines | get 1) "host: aarch64-apple-darwin"

  rm -rf $tmp
}

def test_shim_idempotent_install [] {
  let tmp = (mktemp -d)
  let bin = ($tmp | path join "rust" "bin")
  mkdir $bin
  let real = ($bin | path join "rustc")
  let real_body = "#!/bin/sh
echo 'rustc 1.89.0-dev'"
  $real_body | save --force $real
  chmod +x $real

  install-rustc-shim $tmp "1.52"
  install-rustc-shim $tmp "1.52"

  # `rustc.real` must still contain the original body — second install must
  # not clobber it by moving the shim onto itself.
  let real_after = (open --raw ($bin | path join "rustc.real") | str trim)
  assert equal $real_after $real_body

  rm -rf $tmp
}

# ensure-sbf-sdk must produce a writable copy of the upstream SDK with the
# dependencies/ tree pre-populated so the SDK's install.sh skips every
# download path.
def test_ensure_sbf_sdk_populates_dependencies [] {
  let tmp = (mktemp -d)
  # Fake "upstream" SDK with scripts/install.sh, scripts/strip.sh, env.sh.
  let src = ($tmp | path join "src-sdk")
  mkdir ($src | path join "scripts")
  "#!/usr/bin/env bash
echo install" | save ($src | path join "scripts" "install.sh")
  "#!/usr/bin/env bash
echo strip" | save ($src | path join "scripts" "strip.sh")
  "echo env"     | save ($src | path join "env.sh")
  # Fake platform-tools tree (any non-empty dir will do for symlink target).
  let pt = ($tmp | path join "pt")
  mkdir ($pt | path join "rust" "bin")
  "rustc"        | save ($pt | path join "rust" "bin" "rustc")
  # Destination must NOT exist beforehand; ensure-sbf-sdk creates it.
  let dest = ($tmp | path join "out-sdk")

  ensure-sbf-sdk $src $dest $pt "1.51"

  assert ($dest | path exists)
  # SDK contents copied.
  assert ($dest | path join "env.sh" | path exists)
  # dependencies/ subtree pre-populated.
  let deps = ($dest | path join "dependencies")
  assert ($deps | path join "platform-tools" | path exists)
  assert equal (readlink ($deps | path join "platform-tools") | str trim) $pt
  assert ($deps | path join "platform-tools-v1.51.md" | path exists)
  assert ($deps | path join "criterion" | path exists)
  assert ($deps | path join "criterion-v2.3.2.md" | path exists)
  assert ($deps | path join "criterion-v2.3.3.md" | path exists)
  # Shebangs in scripts/*.sh must be patched away from `/usr/bin/env bash`
  # and a PATH= line must be injected so coreutils are visible when
  # cargo-build-sbf spawns them with an empty environment.
  let install_lines = (open --raw ($dest | path join "scripts" "install.sh") | lines)
  assert equal ($install_lines | first) "#!/bin/bash"
  assert ($install_lines | get 1 | str starts-with "export PATH=")
  let strip_lines = (open --raw ($dest | path join "scripts" "strip.sh") | lines)
  assert equal ($strip_lines | first) "#!/bin/bash"
  assert ($strip_lines | get 1 | str starts-with "export PATH=")

  rm -rf $tmp
}

def test_ensure_sbf_sdk_repopulates_when_source_changes [] {
  let tmp = (mktemp -d)
  let src_a = ($tmp | path join "a")
  let src_b = ($tmp | path join "b")
  mkdir ($src_a | path join "scripts")
  mkdir ($src_b | path join "scripts")
  "A" | save ($src_a | path join "marker")
  "B" | save ($src_b | path join "marker")
  let pt = ($tmp | path join "pt")
  mkdir $pt
  let dest = ($tmp | path join "dest-sdk")

  ensure-sbf-sdk $src_a $dest $pt "1.51"
  let first = (open --raw ($dest | path join "marker") | str trim)
  assert equal $first "A"

  ensure-sbf-sdk $src_b $dest $pt "1.51"
  let second = (open --raw ($dest | path join "marker") | str trim)
  assert equal $second "B"

  rm -rf $tmp
}

def main [] {
  test_strip_build_sbf_drops_leading_subcommand
  test_strip_build_sbf_passes_through_when_absent
  test_strip_build_sbf_handles_empty
  test_strip_build_sbf_only_strips_first_position
  test_ensure_populates_when_missing
  test_ensure_no_op_when_marker_matches
  test_ensure_repopulates_on_source_change
  test_shim_adds_toolchain_marker_to_version
  test_shim_adds_marker_for_short_verbose_version
  test_shim_adds_marker_for_separate_verbose_flag
  test_shim_forwards_other_args
  test_shim_works_with_stripped_path
  test_shim_idempotent_install
  test_ensure_sbf_sdk_populates_dependencies
  test_ensure_sbf_sdk_repopulates_when_source_changes
  print "cargo-build-sbf.test.nu: all tests passed"
}
