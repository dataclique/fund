{ rustToolchain }:

{
  # Nix
  nil.enable = true;
  nixfmt.enable = true;

  # Rust
  rustfmt = {
    enable = true;
    entry = "${rustToolchain}/bin/cargo fmt --";
    files = "\\.rs$";
    pass_filenames = true;
  };

  # TypeScript / JS / JSON
  prettier = {
    enable = true;
    excludes = [ "\\.md$" ];
  };

  # TOML
  taplo.enable = true;
}
