#!/usr/bin/env nu
#
# Project-local wrapper for `cargo-build-sbf` (from `solana-cli`).
#
# Wires up an *offline* anchor build:
#
#   1. Materialize a writable copy of the nix-prefetched `platform-tools/`
#      under `$CARGO_BUILD_SBF_HOME/.cache/solana/v<X>/platform-tools/`.
#   2. Replace `platform-tools/rust/bin/rustc` with a tiny shim that appends
#      ` toolchain-v<X>` to the `rustc --version` line. solana-cli's
#      cargo-build-sbf validates the toolchain with the regex
#      `(rustc [0-9]+\.[0-9]+\.[0-9]+).*toolchain-v`; the released
#      platform-tools rustc does not emit that marker, so without this the
#      `--skip-tools-install` validation reports "Solana toolchain is
#      corrupted". For any non-`--version` invocation the shim execs the
#      real rustc, so cargo compilation is unaffected.
#   3. Prepend `platform-tools/rust/bin/` to `PATH` so cargo-build-sbf and
#      the cargo it spawns both find the shimmed Solana rustc (which has
#      the sbpf-solana-solana target).
#   4. Run `cargo-build-sbf` with `--skip-tools-install --no-rustup-override`
#      so it never touches the network and never tries to push a `+solana`
#      rustup toolchain (rustup isn't in the dev shell).
#
# Required environment (set by the nix wrapper in flake.nix):
#   CARGO_BUILD_SBF_REAL_BIN              solana-cli's cargo-build-sbf
#   CARGO_BUILD_SBF_HOME                  project-local HOME for cargo-build-sbf
#   CARGO_BUILD_SBF_PLATFORM_TOOLS        nix store path with extracted v<X> tools
#   CARGO_BUILD_SBF_PLATFORM_TOOLS_VERSION  e.g. "1.52"

# Pure helper, exported so the test suite can exercise it.
export def strip-build-sbf [args: list<string>]: nothing -> list<string> {
  if (($args | length) > 0) and (($args | first) == "build-sbf") {
    $args | skip 1
  } else {
    $args
  }
}

# Populate `$cache_dir` with a writable copy of `$source` if missing or stale.
# A `.source` marker records which nix store path the contents came from so a
# version bump in flake.nix triggers a re-populate.
export def ensure-platform-tools [source: string, cache_dir: string] {
  let marker = ($cache_dir | path join ".source")
  let stale = (
    (not ($cache_dir | path exists))
    or (not ($marker | path exists))
    or ((open --raw $marker | str trim) != $source)
  )
  if $stale {
    if ($cache_dir | path exists) { rm -rf $cache_dir }
    let parent = ($cache_dir | path dirname)
    mkdir $parent
    cp -r $source $cache_dir
    chmod -R u+w $cache_dir
    $source | save --force $marker
  }
}

# Materialize a writable copy of the platform-tools SDK at `$cache_dir`'s
# sibling and pre-populate its `dependencies/` directory so that
# `${SDK}/scripts/install.sh` (invoked by `${SDK}/env.sh` on every build)
# becomes a no-op:
#   - dependencies/platform-tools  → symlinked to the prefetched nix copy
#   - dependencies/platform-tools-v<X>.md  → empty marker
#   - dependencies/criterion       → empty dir (we don't build C tests)
#   - dependencies/criterion-v2.3.2.md / v2.3.3.md → empty markers
#
# The original SDK lives in the nix store and is read-only, so
# `mkdir -p .../dependencies` from install.sh fails with EACCES. Copying
# the SDK into `.devenv/sbf-home/` is the simplest fix.
export def ensure-sbf-sdk [
  source_sdk: string                # nix store path of the upstream platform-tools-sdk/sbf
  sdk_dir: string                   # writable destination, e.g. $SBF_HOME/sbf-sdk
  platform_tools: string            # writable platform-tools dir (already populated)
  tools_version: string             # "1.51"
] {
  let marker = ($sdk_dir | path join ".source")
  let stale = (
    (not ($sdk_dir | path exists))
    or (not ($marker | path exists))
    or ((open --raw $marker | str trim) != $source_sdk)
  )
  if $stale {
    if ($sdk_dir | path exists) { rm -rf $sdk_dir }
    let parent = ($sdk_dir | path dirname)
    mkdir $parent
    cp -r $source_sdk $sdk_dir
    chmod -R u+w $sdk_dir
    $source_sdk | save --force $marker
    # The SDK's scripts use `#!/usr/bin/env bash`. On macOS, `env`'s
    # lookup of `bash` fails with `path too long` when the dev shell's
    # PATH is huge (every nixpkgs bin dir). And cargo-build-sbf spawns
    # `strip.sh` with an *empty* PATH, so even after fixing the shebang
    # the script can't find `dirname`/`mkdir`. Patch both: rewrite the
    # shebang to absolute `/bin/bash` and inject a PATH= line that points
    # at host coreutils so the rest of the script runs.
    let coreutils_bin = (which dirname | get path? | first)
    if ($coreutils_bin | is-empty) {
      error make { msg: "no `dirname` found on PATH — wrapper runtimeInputs must include coreutils" }
    }
    let coreutils_dir = ($coreutils_bin | path dirname)
    let scripts_dir = ($sdk_dir | path join "scripts")
    for script in (glob ($scripts_dir | path join "*.sh")) {
      let content = (open --raw $script)
      let stripped = ($content | str replace "#!/usr/bin/env bash" "")
      let patched = $"#!/bin/bash
export PATH=\"($coreutils_dir):$PATH\"
($stripped)"
      $patched | save --force $script
      chmod +x $script
    }
  }
  let deps = ($sdk_dir | path join "dependencies")
  mkdir $deps
  let pt_link = ($deps | path join "platform-tools")
  if ($pt_link | path exists) { rm -rf $pt_link }
  ^ln -s $platform_tools $pt_link
  touch ($deps | path join $"platform-tools-v($tools_version).md")
  # install.sh wants Criterion too, even though we don't use it. Pre-create
  # both possible markers (linux/darwin disagree) plus an empty `criterion`
  # directory so the existence check passes.
  mkdir ($deps | path join "criterion")
  touch ($deps | path join "criterion-v2.3.2.md")
  touch ($deps | path join "criterion-v2.3.3.md")
}

# Install the rustc `--version` shim at `$cache_dir/rust/bin/rustc`.
# Idempotent — the original rustc is moved aside to `rustc.real` once, then
# subsequent calls re-overwrite the shim only if its content drifted.
export def install-rustc-shim [cache_dir: string, version: string] {
  let bin_dir = ($cache_dir | path join "rust" "bin")
  let rustc = ($bin_dir | path join "rustc")
  let real_rustc = ($bin_dir | path join "rustc.real")
  if not ($real_rustc | path exists) {
    mv $rustc $real_rustc
  }
  let nl = (char newline)
  let shim = $"#!/bin/sh
# Auto-generated by cargo-build-sbf.nu — see scripts/cargo-build-sbf.nu.
# Adds the `toolchain-v($version)` marker that solana-cli's cargo-build-sbf
# greps for in rustc's version output. Forwards anything that isn't a
# version probe verbatim to the real rustc.
#
# IMPORTANT: this script uses POSIX shell builtins only. `cargo` and
# `cargo-build-sbf` invoke rustc with a stripped PATH, so external tools
# like `head` / `tail` / `awk` are NOT available here.
if [ -n \"$RUSTC_SHIM_LOG\" ]; then
  printf 'rustc-shim: %s\\n' \"$*\" >> \"$RUSTC_SHIM_LOG\"
fi

# cargo-build-sbf has been observed to ask the version via `--version`,
# `--version --verbose`, and `-vV`. Detect any args list that's purely a
# version probe and append the toolchain marker to the first output line.
is_version_probe=1
for arg in \"$@\"; do
  case \"$arg\" in
    --version|--verbose|-V|-v|-vV|-Vv) ;;
    *) is_version_probe=0 ;;
  esac
done
if [ \"$#\" -gt 0 ] && [ \"$is_version_probe\" = \"1\" ]; then
  output=\"$\(($real_rustc) \"$@\"\)\" || exit $?
  nl='($nl)'
  case \"$output\" in
    *\"$nl\"*)
      first=\"${output%%\"$nl\"*}\"
      rest=\"${output#*\"$nl\"}\"
      printf '%s toolchain-v($version)\\n%s\\n' \"$first\" \"$rest\"
      ;;
    *)
      printf '%s toolchain-v($version)\\n' \"$output\"
      ;;
  esac
  exit 0
fi
exec ($real_rustc) \"$@\"
"
  $shim | save --force $rustc
  chmod +x $rustc
}

def --wrapped main [...args: string] {
  let real_bin = ($env.CARGO_BUILD_SBF_REAL_BIN? | default "")
  let sbf_home = ($env.CARGO_BUILD_SBF_HOME? | default "")
  let tools = ($env.CARGO_BUILD_SBF_PLATFORM_TOOLS? | default "")
  let tools_version = ($env.CARGO_BUILD_SBF_PLATFORM_TOOLS_VERSION? | default "")
  let source_sdk = ($env.CARGO_BUILD_SBF_SOURCE_SDK? | default "")
  if (
    ($real_bin == "") or ($sbf_home == "") or ($tools == "")
    or ($tools_version == "") or ($source_sdk == "")
  ) {
    error make { msg: "CARGO_BUILD_SBF_REAL_BIN / _HOME / _PLATFORM_TOOLS / _PLATFORM_TOOLS_VERSION / _SOURCE_SDK must all be set" }
  }
  let cache_dir = ($sbf_home | path join ".cache" "solana" $"v($tools_version)" "platform-tools")
  ensure-platform-tools $tools $cache_dir
  install-rustc-shim $cache_dir $tools_version
  let sdk_dir = ($sbf_home | path join "sbf-sdk")
  ensure-sbf-sdk $source_sdk $sdk_dir $cache_dir $tools_version
  let bin_dir = ($cache_dir | path join "rust" "bin")
  let forwarded = strip-build-sbf $args
  # The platform-tools rustc defaults to `cc` for the host-target linker.
  # cargo-build-sbf strips `RUSTFLAGS` from the cargo env (it explicitly
  # logs `Removed RUSTFLAGS from cargo environment`), so a global
  # `-C linker=...` won't survive. The per-target
  # `CARGO_TARGET_<TARGET>_LINKER` and `CARGO_TARGET_<TARGET>_RUSTFLAGS`
  # variables ARE preserved, so we use those to pin the host linker to
  # the nixpkgs cc-wrapper (`pkgs.stdenv.cc` is in the wrapper's
  # runtimeInputs). `cc-rs`-style `CC_<target>` covers C build deps.
  let host_cc = (which cc | get path? | first)
  if ($host_cc | is-empty) {
    error make { msg: "no `cc` found on PATH — the wrapper's runtimeInputs is missing stdenv.cc" }
  }
  with-env {
    HOME: $sbf_home
    PATH: $"($bin_dir):($env.PATH)"
    SBF_SDK_PATH: $sdk_dir
    CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER: $host_cc
    CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER: $host_cc
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER: $host_cc
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER: $host_cc
    CC_aarch64_apple_darwin: $host_cc
    CC_x86_64_apple_darwin: $host_cc
    CC_aarch64_unknown_linux_gnu: $host_cc
    CC_x86_64_unknown_linux_gnu: $host_cc
  } {
    ^$real_bin --skip-tools-install --no-rustup-override ...$forwarded
  }
}
