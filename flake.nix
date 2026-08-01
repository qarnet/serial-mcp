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
    nix-nrf-dev = {
      url = "github:qarnet/nix-nrf-dev";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
      flake-utils,
      nix-nrf-dev,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
          };
        };

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
            || (pkgs.lib.hasPrefix "/schemas" relPath && !pkgs.lib.hasSuffix "opencode.schema.json" relPath)
            || pkgs.lib.hasPrefix "/example-configs" relPath
            # Test fixtures read via CARGO_MANIFEST_DIR must survive the
            # source filter: doc_drift reads README.md, server.json, and
            # docs/ (agent-config.md, development/FEATURES.md, future
            # evaluations); config_schema_validation reads schemas/ and
            # example-configs/. crane runs `cargo test` during `nix flake
            # check` — a pruned fixture fails the build (doc_drift) or
            # silently skips the checks (config_schema_validation).
            # relPath keeps a leading "/", hence the explicit "/" in every
            # prefix below; a directory must itself match the filter or
            # cleanSource prunes its whole subtree, so dirs are included
            # as whole trees to spare future fixture edits.
            #
            # Exception: schemas/opencode.schema.json refs
            # https://models.dev/model-schema.json, which jsonschema fetches
            # eagerly — the network-less Nix sandbox can't resolve it, so
            # that file stays out of the Nix source (its fixture still
            # silently skips there; network-enabled CI covers it). Vendoring
            # the models.dev schema is tracked in FEATURES.md
            # (Infrastructure / tech debt); remove this exclusion when it
            # lands.
            || pkgs.lib.hasSuffix "README.md" relPath
            || pkgs.lib.hasSuffix "server.json" relPath
            || pkgs.lib.hasPrefix "/docs" relPath;
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

        # Source-matched MCP server executable for the dev shell. Referencing
        # ${serial-mcp} forces the current source derivation to be realized
        # during shell activation, so OpenCode can start the server directly
        # instead of triggering a (potentially cold) `nix run` build inside
        # its MCP initialization deadline.
        serial-mcp-dev = pkgs.writeShellScriptBin "serial-mcp-dev" ''
          exec ${serial-mcp}/bin/serial-mcp "$@"
        '';

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
          config = {
            allowUnfree = true;
          };
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
          serial-mcp-dev = flake-utils.lib.mkApp {
            drv = serial-mcp;
            name = "serial-mcp";
          };
        };

        # `nix develop`
        #
        # Hybrid shell: nix-nrf-dev's mkNrfShell owns the Nordic environment
        # (sdk-manager variables scoped to the west wrapper) and the multilib
        # GCC for native_sim; crane/Rust inputs and project tools come from
        # this flake.
        devShells.default = nix-nrf-dev.lib.${system}.mkNrfShell {
          name = "serial-mcp";
          ncsVersion = "v3.3.0";
          withMultilib = true;

          # Inherit nativeBuildInputs/buildInputs/env vars from the package.
          inputsFrom = [ serial-mcp ];

          # craneLib.devShell no longer injects the Rust toolchain into the
          # shell; list it explicitly. Extras only useful at dev time, not
          # for builds.
          packages =
            (with pkgs; [
              rustToolchain
              cargo-watch
              cargo-edit
              cargo-nextest
              jsonschema-cli
              mcp-publisher
            ])
            ++ [ serial-mcp-dev ];

          extraShellHook = ''
            export PATH="$PWD/scripts:$PWD/firmware/bin:$PATH"
            echo "serial-mcp dev shell"
            echo "rustc: $(rustc --version)"
            if command -v west >/dev/null 2>&1; then
              echo "west: $(west --version 2>/dev/null | head -n 1)"
            fi
          '';
        };

        # `nix flake check`
        #
        # Only the package build is checked here. fmt, clippy, and the test
        # suite are all run by the build/test/clippy matrix in
        # .github/workflows/ci.yml on 4 OSes (ubuntu-latest,
        # ubuntu-24.04-arm, macos-14, windows-latest) plus the native-sim job.
        # Re-running them via Nix duplicated that work (~10 min of redundant
        # crate compilation + test execution) without adding coverage — the
        # unique value of `nix flake check` is verifying the flake itself is
        # valid and the nixpkgs derivation builds. Keep it to that.
        checks = {
          inherit serial-mcp;
        };
      }
    );
}
