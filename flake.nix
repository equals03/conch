{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = {
    nixpkgs,
    crane,
    rust-overlay,
    advisory-db,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default);

      # Include non-Rust fixture and golden files next to common Cargo sources so tests see them
      # in the sandbox (`cleanCargoSource` omits e.g. `.yaml`, `.json`, `.fish`, `.bash`).
      src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          (craneLib.fileset.commonCargoSources ./.)
          (pkgs.lib.fileset.fileFilter (
              file:
                file.hasExt "yaml"
                || file.hasExt "yml"
                || file.hasExt "json"
                || file.hasExt "fish"
                || file.hasExt "bash"
            )
            ./.)
        ];
      };

      common-args = {
        inherit src;
        strictDeps = true;
      };
      cargo-artifacts = craneLib.buildDepsOnly common-args;
      conch = craneLib.buildPackage (common-args
        // {
          cargoArtifacts = cargo-artifacts;
        });

      app = flake-utils.lib.mkApp {
        drv = conch;
      };
    in {
      checks = {
        inherit conch;

        # Audit dependencies
        conch-output-lock-audit = craneLib.cargoAudit {
          inherit src advisory-db;
        };

        # Audit licenses
        conch-output-lock-deny = craneLib.cargoDeny {
          inherit src;
        };
      };
      packages = {
        inherit conch;
        default = conch;
      };
      apps = {
        conch = app;
        default = app;
      };

      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          (
            rust-bin.stable.latest.default.override {
              extensions = ["rust-src"];
            }
          )
        ];
      };
    });
}
