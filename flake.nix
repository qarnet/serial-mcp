{
  description = "serial-mcp dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Pinned via rust-toolchain.toml. Includes rust-src + rust-analyzer
        # because we declare them in that file (see below).
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            let
              relPath = pkgs.lib.removePrefix (toString ./.) (toString path);
            in
            craneLib.filterCargoSources path type
            || pkgs.lib.hasPrefix "schemas" relPath
            || pkgs.lib.hasPrefix "example-configs" relPath;
        };

        # Common args shared by both the deps-only and final derivations.
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [
            udev
            openssl
          ];
        };

        # Build *just* the dependencies. This output gets cached and reused
        # as long as Cargo.lock doesn't change — so changes to your own code
        # only rebuild your own crate.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # ─── mcp-publisher (pre-built binary from GitHub releases) ────────────
        mcpPublisherVersion = "1.7.9";
        mcpPublisherSrc =
          {
            x86_64-linux = {
              suffix = "linux_amd64";
              hash = "sha256-qxKBYrBhYJC0fPJFr+CiPz7wiTb9zhkHT1ugpEaSgaw=";
            };
            aarch64-linux = {
              suffix = "linux_arm64";
              hash = "sha256-BPUZmz3u+Ob8TW7ZjFanT3md71Ptyj/m1IYuzUOXwXI=";
            };
            x86_64-darwin = {
              suffix = "darwin_amd64";
              hash = "sha256-glC2HHUwlg+7VPmdqpEAEATjZcYEyzBbE/wHLqP1zKk=";
            };
            aarch64-darwin = {
              suffix = "darwin_arm64";
              hash = "sha256-WSXI0slCsqAzC5eVMLXXAoTDvbA4UKPNEDJoW4DdwuM=";
            };
          }
          .${system} or (throw "mcp-publisher: unsupported system ${system}");

        mcp-publisher = pkgs.stdenvNoCC.mkDerivation {
          pname = "mcp-publisher";
          version = mcpPublisherVersion;
          src = pkgs.fetchurl {
            url = "https://github.com/modelcontextprotocol/registry/releases/download/v${mcpPublisherVersion}/mcp-publisher_${mcpPublisherSrc.suffix}.tar.gz";
            hash = mcpPublisherSrc.hash;
          };
          # The tarball has no subdirectory; extract manually to avoid sourceRoot issues.
          dontUnpack = true;
          # Patch the ELF interpreter on Linux so glibc is found via the Nix store.
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.autoPatchelfHook ];
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.glibc ];
          installPhase = ''
            runHook preInstall
            mkdir -p $out/bin
            tar -xOf $src mcp-publisher > $out/bin/mcp-publisher
            chmod +x $out/bin/mcp-publisher
            runHook postInstall
          '';
        };

        serial-mcp = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        # ─── Cross-compilation: aarch64-unknown-linux-gnu ──────────────────
        # Only meaningful when building from x86_64-linux.
        pkgsCross = import nixpkgs {
          inherit system overlays;
          crossSystem.config = "aarch64-unknown-linux-gnu";
        };

        craneLibCross = (crane.mkLib pkgsCross).overrideToolchain rustToolchain;

        serial-mcp-aarch64 = craneLibCross.buildPackage {
          inherit src;
          strictDeps = true;

          # Tools that run on the BUILD machine (x86_64 here).
          nativeBuildInputs = with pkgs; [ pkg-config ];
          depsBuildBuild = [ pkgsCross.stdenv.cc ];

          # Libraries linked into the TARGET binary (aarch64).
          buildInputs = with pkgsCross; [
            udev
            openssl
          ];

          CARGO_BUILD_TARGET = "aarch64-unknown-linux-gnu";
          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${pkgsCross.stdenv.cc.targetPrefix}cc";

          # pkg-config must look in the cross sysroot, not the host one.
          PKG_CONFIG_PATH = "${pkgsCross.udev.dev}/lib/pkgconfig";
          PKG_CONFIG_ALLOW_CROSS = "1";
        };
      in
      {
        # `nix build`, `nix run github:qarnet/serial-mcp`
        packages = {
          default = serial-mcp;
          serial-mcp = serial-mcp;
          serial-mcp-aarch64 = serial-mcp-aarch64;
          inherit mcp-publisher;
        };

        # `nix run .#<name>` — entry points for each binary.
        apps = {
          default = flake-utils.lib.mkApp {
            drv = serial-mcp;
            name = "serial-mcp";
          };
        };

        # `nix develop`
        devShells.default = craneLib.devShell {
          # Inherit nativeBuildInputs/buildInputs/env vars from the package.
          inputsFrom = [ serial-mcp ];

          # Extras only useful at dev time, not for builds.
          packages = with pkgs; [
            cargo-watch
            cargo-edit
            cargo-nextest
            jsonschema-cli
            mcp-publisher
          ];

          env.RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "serial-mcp dev shell"
            echo "rustc: $(rustc --version)"
          '';
        };

        # `nix flake check`
        checks = {
          inherit serial-mcp;

          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          fmt = craneLib.cargoFmt {
            src = commonArgs.src;
          };

          nextest = craneLib.cargoNextest (
            commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
            }
          );
        };
      }
    );
}
