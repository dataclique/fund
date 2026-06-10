{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";

    devenv.url = "github:cachix/devenv";
    devenv.inputs = {
      nixpkgs.follows = "nixpkgs";
      git-hooks.follows = "git-hooks";
    };

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    but = {
      url = "github:dataclique/but.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      git-hooks,
      devenv,
      rust-overlay,
      but,
      ...
    }@inputs:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        hooks = import ./hooks.nix { inherit rustToolchain; };

        # cargo-build-sbf from `solana-cli` 3.0.12 is hardcoded against
        # platform-tools v1.51 (rustc 1.84.1) — newer versions trip an
        # internal toolchain-version assertion and fail with "Solana
        # toolchain is corrupted". See `AGENTS.md` for details.
        platformToolsVersion = "1.51";
        platformToolsAssets = {
          "aarch64-darwin" = {
            name = "platform-tools-osx-aarch64.tar.bz2";
            sha256 = "1cvcdrx5y9ldiprpj4nggb9dnaqjq0zc90fsfvx9k0gy6wqjpqx1";
          };
        };
        platformToolsAsset = platformToolsAssets.${system} or null;
        platformTools =
          if platformToolsAsset == null then
            null
          else
            pkgs.runCommandLocal "solana-platform-tools-v${platformToolsVersion}"
              {
                src = pkgs.fetchurl {
                  url = "https://github.com/anza-xyz/platform-tools/releases/download/v${platformToolsVersion}/${platformToolsAsset.name}";
                  inherit (platformToolsAsset) sha256;
                };
                nativeBuildInputs = [
                  pkgs.gnutar
                  pkgs.bzip2
                ];
              }
              ''
                mkdir -p $out
                tar -xjf $src -C $out
              '';

        # Ships all `./scripts/*.nu` files to `$out/libexec/` after running
        # each `*.test.nu` in checkPhase. Used by the cargo-build-sbf wrapper
        # and by the probe scripts below.
        sbfScripts = pkgs.stdenv.mkDerivation {
          pname = "fund-sbf-scripts";
          version = "0.1.0";
          src = ./scripts;
          nativeBuildInputs = [ pkgs.nushell ];
          doCheck = true;
          checkPhase = ''
            for t in ./*.test.nu; do
              echo "running $t"
              nu "$t"
            done
          '';
          installPhase = ''
            mkdir -p $out/libexec
            for f in ./*.nu; do
              cp "$f" $out/libexec/
            done
          '';
        };

        cargoBuildSbfWrapper =
          if platformTools == null then
            null
          else
            pkgs.writeShellApplication {
              name = "cargo-build-sbf";
              runtimeInputs = [
                pkgs.nushell
                # Host build scripts (rustc → `cc`) need a wrapped C
                # compiler on PATH. `stdenv.cc` provides `cc` / `clang` /
                # the right linker flags for the current platform.
                pkgs.stdenv.cc
              ];
              text = ''
                export CARGO_BUILD_SBF_REAL_BIN="${pkgs.solana-cli}/bin/cargo-build-sbf"
                export CARGO_BUILD_SBF_HOME="''${DEVENV_ROOT:-$PWD}/.devenv/sbf-home"
                export CARGO_BUILD_SBF_PLATFORM_TOOLS="${platformTools}"
                export CARGO_BUILD_SBF_PLATFORM_TOOLS_VERSION="${platformToolsVersion}"
                export CARGO_BUILD_SBF_SOURCE_SDK="${pkgs.solana-cli}/bin/platform-tools-sdk/sbf"
                exec nu ${sbfScripts}/libexec/cargo-build-sbf.nu "$@"
              '';
            };

        # Diagnostic probes. Both are real derivations exposed on the dev
        # shell PATH so you can run e.g. `probe-cargo-build-sbf --clean
        # --manifest-path programs/fund/Cargo.toml` directly — no need for
        # `nix develop --impure -- nu …`.
        probeCargoBuildSbf =
          if cargoBuildSbfWrapper == null then
            null
          else
            pkgs.writeShellApplication {
              name = "probe-cargo-build-sbf";
              runtimeInputs = [
                pkgs.nushell
                cargoBuildSbfWrapper
              ];
              text = ''
                exec nu ${sbfScripts}/libexec/probe-cargo-build-sbf.nu "$@"
              '';
            };

        probeRustcShim =
          if cargoBuildSbfWrapper == null then
            null
          else
            pkgs.writeShellApplication {
              name = "probe-rustc-shim";
              runtimeInputs = [
                pkgs.nushell
                cargoBuildSbfWrapper
              ];
              text = ''
                exec nu ${sbfScripts}/libexec/probe-rustc-shim.nu "$@"
              '';
            };

        regenerateCargoLockSbf =
          if cargoBuildSbfWrapper == null then
            null
          else
            pkgs.writeShellApplication {
              name = "regenerate-cargo-lock-sbf";
              runtimeInputs = [
                pkgs.nushell
                cargoBuildSbfWrapper
              ];
              text = ''
                exec nu ${sbfScripts}/libexec/regenerate-cargo-lock-sbf.nu "$@"
              '';
            };

        rustPlatformPinned = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        # `anchor idl build` invokes `cargo +<toolchain> test ...` (rustup
        # syntax) and falls back to running `rustup toolchain install` when
        # that fails. A nix-provided cargo rejects `+<toolchain>` and there
        # is no rustup, so the build dies with ENOENT. These shims strip the
        # `+<toolchain>` argument and satisfy the rustup probe.
        cargoRustupArgShim = pkgs.writeShellApplication {
          name = "cargo";
          text = ''
            # ''${1#+} strips a leading '+'; if that changes $1, then $1 is a
            # rustup +<toolchain> selector (e.g. +1.95.0) the nix cargo rejects,
            # so drop it before exec'ing the real cargo.
            if [ "''${1:-}" != "''${1#+}" ]; then
              shift
            fi
            exec ${rustToolchain}/bin/cargo "$@"
          '';
        };

        rustupNoopShim = pkgs.writeShellApplication {
          name = "rustup";
          text = ''
            exit 0
          '';
        };

        # The program's Anchor IDL, generated with the host toolchain via
        # `anchor idl build` (no SBF build, no platform-tools), exposed so
        # consumers can take this flake as an input and generate client
        # bindings from `$out/idl/fund.json`.
        fundIdl = rustPlatformPinned.buildRustPackage {
          pname = "fund-idl";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [
            cargoRustupArgShim
            rustupNoopShim
            pkgs.anchor
            pkgs.pkg-config
          ];

          buildInputs = [ pkgs.openssl ];

          buildPhase = ''
            runHook preBuild
            # `-- --lib` restricts the underlying `cargo test` to the lib
            # target: the IDL print-tests are generated into the lib, while
            # the litesvm integration tests `include_bytes!` the SBF artifact
            # (target/deploy/fund.so), which this derivation deliberately
            # does not build. Note this couples IDL export to the lib unit
            # tests passing -- acceptable while those tests are host-pure.
            anchor idl build --program-name fund --out fund-idl.json -- --lib
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            mkdir -p $out/idl
            cp fund-idl.json $out/idl/fund.json
            runHook postInstall
          '';

          doCheck = false;
        };

      in
      {
        devShells.default = devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [
            (but.lib.${system}.devenvModule {
              repoNotes = ''
                ## This Repository

                Commit, branch, and pre-commit-hook conventions live in
                `AGENTS.md` under "Version control (GitButler)" -- that
                section is the source of truth; read it before committing.

              '';
            })
            {
              packages =
                pkgs.lib.optional (cargoBuildSbfWrapper != null) cargoBuildSbfWrapper
                ++ pkgs.lib.optional (probeCargoBuildSbf != null) probeCargoBuildSbf
                ++ pkgs.lib.optional (probeRustcShim != null) probeRustcShim
                ++ pkgs.lib.optional (regenerateCargoLockSbf != null) regenerateCargoLockSbf
                ++ (with pkgs; [
                  anchor
                  solana-cli
                  pkg-config
                  openssl
                  nushell
                ]);

              languages = {
                nix.enable = true;
                javascript = {
                  enable = true;
                  bun = {
                    enable = true;
                    install.enable = true;
                  };
                };

                rust = {
                  enable = true;
                  toolchain = {
                    rustc = rustToolchain;
                    cargo = rustToolchain;
                    rustfmt = rustToolchain;
                    clippy = rustToolchain;
                  };
                };
              };

              git-hooks = { inherit hooks; };

              difftastic.enable = true;
              cachix.enable = true;
            }
          ];
        };

        packages = {
          idl = fundIdl;
          # `default` is the IDL, not the SBF program: the SBF build needs the
          # cargo-build-sbf shim + platform-tools and is not a pure derivation
          # (see the SBF toolchain notes in AGENTS.md), so `nix build .` yields
          # the IDL that downstream consumers actually take this flake for.
          default = fundIdl;
        };

        checks = {
          git-hooks = git-hooks.lib.${system}.run {
            inherit hooks;
            src = self;
          };
          inherit sbfScripts;
          idl = fundIdl;
        };

        # Diagnostic probes exposed as flake apps so they can be invoked
        # from outside the dev shell:
        #   nix run .#probe-cargo-build-sbf -- --clean --manifest-path …
        #   nix run .#probe-rustc-shim -- --tools-version 1.51
        apps = pkgs.lib.optionalAttrs (cargoBuildSbfWrapper != null) {
          probe-cargo-build-sbf = {
            type = "app";
            program = "${probeCargoBuildSbf}/bin/probe-cargo-build-sbf";
          };
          probe-rustc-shim = {
            type = "app";
            program = "${probeRustcShim}/bin/probe-rustc-shim";
          };
          regenerate-cargo-lock-sbf = {
            type = "app";
            program = "${regenerateCargoLockSbf}/bin/regenerate-cargo-lock-sbf";
          };
        };
      }
    );

  nixConfig = {
    extra-substituters = [
      "https://devenv.cachix.org"
      "https://nix-community.cachix.org"
    ];
    extra-trusted-public-keys = [
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
    ];
    allow-unfree = true;
  };
}
