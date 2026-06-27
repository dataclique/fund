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

  # Markdown (prettier excludes it above; deno fmt owns markdown formatting).
  # AGENTS.md (and its CLAUDE.md symlink) is excluded for now: it is edited
  # concurrently by several in-flight branches, so reflowing the whole file
  # cannot be committed to any one of them without conflicts. Drop this
  # exclusion and format AGENTS.md once those branches land.
  denofmt = {
    enable = true;
    files = "\\.md$";
    excludes = [
      "^AGENTS\\.md$"
      "^CLAUDE\\.md$"
    ];
  };

  # TOML
  taplo.enable = true;
}
