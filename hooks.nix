{ rustToolchain }:

{
  # Nix
  nil.enable = true;
  nixfmt.enable = true;
  statix.enable = true;
  deadnix.enable = true;

  # Rust
  rustfmt = {
    enable = true;
    entry = "${rustToolchain}/bin/cargo fmt";
    files = "\\.rs$";
  };

  # TypeScript / JS / JSON
  prettier = {
    enable = true;
    excludes = [ "\\.md$" ];
  };

  # TOML
  taplo.enable = true;
}
