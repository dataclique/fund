#!/usr/bin/env nu
#
# `anchor idl build` invokes `cargo +<toolchain> test ...` (rustup syntax). A
# nix-provided cargo rejects the `+<toolchain>` selector, so this shim is put on
# PATH as `cargo`: it strips a leading `+<toolchain>` argument (if present) and
# execs the real cargo at $CARGO_REAL_BIN. The flake.nix wrapper sets
# CARGO_REAL_BIN to the pinned rust toolchain's cargo.

# Drop a leading `+<toolchain>` rustup selector (e.g. `+1.95.0`) from the
# argument list. Any other first argument — or no arguments at all — is returned
# unchanged. Only the first position is considered, matching rustup semantics.
export def strip-toolchain [args: list<string>]: nothing -> list<string> {
  if (($args | length) > 0) and (($args | first) | str starts-with "+") {
    $args | skip 1
  } else {
    $args
  }
}

# `--wrapped` makes nu forward every argument — including unknown `--flags`
# like `--features idl-build` — verbatim into `$args` instead of trying to parse
# them as flags of this command. Required for a transparent cargo passthrough.
def --wrapped main [...args: string] {
  let real = ($env.CARGO_REAL_BIN? | default "cargo")
  exec $real ...(strip-toolchain $args)
}
