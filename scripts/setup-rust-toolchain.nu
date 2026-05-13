#!/usr/bin/env nu
#
# Bootstrap a project-local rustup environment for anchor's IDL build.
#
# Why this exists: anchor 1.0.2 (see `idl/src/build.rs` line 146) invokes
# `cargo +<toolchain> test __anchor_private_print_idl …` when generating
# the IDL. The `+<toolchain>` syntax is a *rustup* feature — rustup is a
# single multi-call binary, and `cargo` in a rustup-managed install is a
# symlink back to that same binary which dispatches based on `argv[0]`.
# Our nix dev shell gets `cargo` from `rust-overlay` instead — a direct
# cargo binary that has no idea what `+nix` means and errors out:
#
#     error: invalid value 'nix' for '[TOOLCHAIN]...':
#       invalid toolchain name: 'nix'
#
# Fix: materialize the rustup proxy symlinks (`cargo`, `rustc`, `rustfmt`,
# `clippy`) in a project-local dir and prepend it to PATH so anchor's
# `cargo +nix …` invocation resolves to rustup, which then dispatches to
# the nix-pinned toolchain we registered as "nix" via
# `rustup toolchain link`.
#
# Invoked from `enterShell` via `eval $(nu setup-rust-toolchain.nu …)`
# so the printed `export` lines reach the parent shell.

def main [
  --devenv-root: string      # repo root (DEVENV_ROOT or PWD)
  --rustup-bin: string       # absolute path of the rustup binary
  --rust-toolchain: string   # nix-pinned rust toolchain root
] {
  let rustup_home = ($devenv_root | path join ".devenv" "rustup-home")
  let proxy_bin = ($rustup_home | path join "proxies")
  let toolchains = ($rustup_home | path join "toolchains")

  mkdir $toolchains
  mkdir $proxy_bin

  let linked = ($toolchains | path join "nix")
  if not ($linked | path exists) {
    with-env { RUSTUP_HOME: $rustup_home } {
      ^$rustup_bin toolchain link nix $rust_toolchain
    }
  }

  for tool in ["cargo" "rustc" "rustfmt" "clippy"] {
    let proxy = ($proxy_bin | path join $tool)
    if not ($proxy | path exists) {
      ^ln -s $rustup_bin $proxy
    }
  }

  print $"export RUSTUP_HOME=\"($rustup_home)\""
  print $"export RUSTUP_TOOLCHAIN=nix"
  print $"export PATH=\"($proxy_bin):$PATH\""
}
