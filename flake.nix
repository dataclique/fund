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
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      git-hooks,
      devenv,
      rust-overlay,
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

        cargoBuildSbfScript = pkgs.stdenv.mkDerivation {
          pname = "cargo-build-sbf-script";
          version = "0.1.0";
          src = ./scripts;
          nativeBuildInputs = [ pkgs.nushell ];
          doCheck = true;
          checkPhase = "nu cargo-build-sbf.test.nu";
          installPhase = ''
            mkdir -p $out/libexec
            cp cargo-build-sbf.nu $out/libexec/
          '';
        };

        cargoBuildSbfWrapper = pkgs.writeShellApplication {
          name = "cargo-build-sbf";
          runtimeInputs = [ pkgs.nushell ];
          text = ''
            export CARGO_BUILD_SBF_REAL_BIN="${pkgs.solana-cli}/bin/cargo-build-sbf"
            export CARGO_BUILD_SBF_HOME="''${DEVENV_ROOT:-$PWD}/.devenv/sbf-home"
            exec nu ${cargoBuildSbfScript}/libexec/cargo-build-sbf.nu "$@"
          '';
        };

      in
      {
        devShells.default = devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [
            {
              packages = [
                cargoBuildSbfWrapper
              ]
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

        checks = {
          git-hooks = git-hooks.lib.${system}.run {
            inherit hooks;
            src = self;
          };
          inherit cargoBuildSbfScript;
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
