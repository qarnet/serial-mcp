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

        # Pinned by rust-toolchain.toml; includes rust-src and rust-analyzer
        # declared there.
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
            || pkgs.lib.hasPrefix "/schemas" relPath
            || pkgs.lib.hasPrefix "/example-configs" relPath
            # Keep CARGO_MANIFEST_DIR fixtures in the filtered source:
            # doc_drift reads README.md, server.json, CHANGELOG.md, docs/
            # (agent-config.md, development/FEATURES.md, future evaluations),
            # .github/workflows/ci.yml, conformance/expected-failures.yaml,
            # scripts/inspector-smoke.mjs, and the historical rmcp 1.7 fixture
            # (compat/rmcp-1-client/Cargo.toml + Cargo.lock; policy
            # drift guards). config_schema_validation requires schemas/ and
            # example-configs/; pruning either fixture fails the build.
            # relPath has a leading "/", so every prefix below includes "/".
            # A directory must match the filter or cleanSource prunes its
            # subtree; include directories as whole trees for future fixture
            # edits.
            #
            # All four vendored schemas, including opencode.schema.json and
            # its models.dev resource, validate hermetically offline. The
            # local validator registers models.dev in memory under its
            # original URI; nothing here needs network access.
            || pkgs.lib.hasSuffix "README.md" relPath
            || pkgs.lib.hasSuffix "CHANGELOG.md" relPath
            || pkgs.lib.hasSuffix "server.json" relPath
            || pkgs.lib.hasSuffix "flake.nix" relPath
            || pkgs.lib.hasPrefix "/docs" relPath
            # Keep workflow fixtures and registry-manifest tooling: doc_drift
            # reads .github/workflows at runtime, and builder tests live in
            # scripts/. A pruned fixture fails these checks and CI doc_drift.
            # Match the containing directory (/.github), not only workflows;
            # cleanSource prunes subtrees whose directory does not match.
            || pkgs.lib.hasPrefix "/.github" relPath
            || pkgs.lib.hasPrefix "/conformance" relPath
            || pkgs.lib.hasPrefix "/compat" relPath
            || pkgs.lib.hasPrefix "/scripts" relPath;
        };

        # Arguments shared by dependency-only and final derivations.
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [
            udev
            openssl
          ];
        };

        # Build dependencies only. Cache and reuse output until Cargo.lock
        # changes; own-code changes then rebuild only this crate.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # mcp-publisher is a pre-built binary from GitHub releases.
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
          # Tarball has no subdirectory; extract manually to avoid sourceRoot issues.
          dontUnpack = true;
          # Patch Linux ELF interpreter so glibc is found in the Nix store.
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
        # ${serial-mcp} realizes the current source derivation during shell
        # activation, so OpenCode starts it directly instead of triggering a
        # potentially cold `nix run` build during MCP initialization.
        serial-mcp-dev = pkgs.writeShellScriptBin "serial-mcp-dev" ''
          exec ${serial-mcp}/bin/serial-mcp "$@"
        '';

        serial-mcp = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        # Cross-compilation target: aarch64-unknown-linux-gnu.
        # Relevant only when building from x86_64-linux.
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

          # Tools that run on the build machine (x86_64 here).
          nativeBuildInputs = with pkgs; [ pkg-config ];
          depsBuildBuild = [ pkgsCross.stdenv.cc ];

          # Libraries linked into the target binary (aarch64).
          buildInputs = with pkgsCross; [
            udev
            openssl
          ];

          CARGO_BUILD_TARGET = "aarch64-unknown-linux-gnu";
          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${pkgsCross.stdenv.cc.targetPrefix}cc";

          # Point pkg-config at the cross sysroot, not the host sysroot.
          PKG_CONFIG_PATH = "${pkgsCross.udev.dev}/lib/pkgconfig";
          PKG_CONFIG_ALLOW_CROSS = "1";
        };
      in
      {
        # `nix build`, `nix run github:qarnet/serial-mcp`.
        packages = {
          default = serial-mcp;
          serial-mcp = serial-mcp;
          serial-mcp-aarch64 = serial-mcp-aarch64;
          inherit mcp-publisher;
          # Independent schema validator for the registry manifest. Must never
          # build or depend on the serial-mcp package.
          jsonschema-cli = pkgs.jsonschema-cli;
        };

        # Use `nix run .#<name>` for binary entry points.
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
        # Hybrid shell: nix-nrf-dev's mkNrfShell owns the Nordic environment,
        # scopes sdk-manager variables to the west wrapper, and supplies
        # multilib GCC for native_sim. crane/Rust inputs and project tools
        # come from this flake.
        devShells.default = nix-nrf-dev.lib.${system}.mkNrfShell {
          name = "serial-mcp";
          ncsVersion = "v3.3.0";
          withMultilib = true;

          # Inherit nativeBuildInputs, buildInputs, and environment variables
          # from the package.
          inputsFrom = [ serial-mcp ];

          # craneLib.devShell no longer injects Rust toolchain; list it
          # explicitly. The remaining packages are for development, not
          # builds.
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
        # Check the package build here. fmt, clippy, and tests run in the
        # build/test/clippy matrix in .github/workflows/ci.yml on 4 OSes
        # (ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest) plus
        # native-sim. Repeating them in Nix adds ~10 min of crate compilation
        # and test execution without coverage. The unique value here is
        # validating the flake and nixpkgs derivation. Keep it to that.
        checks = {
          inherit serial-mcp;

          # Prove filtered source ships every workflow fixture doc_drift reads
          # at runtime. If the filter prunes .github/workflows, this check
          # fails.
          workflow-fixtures-present = pkgs.runCommand "workflow-fixtures-present" { } ''
            test -f ${src}/.github/workflows/ci.yml
            test -f ${src}/.github/workflows/hardening.yml
            test -f ${src}/.github/workflows/publish-mcp-registry.yml
            test -f ${src}/.github/workflows/publish-mcp-registry-backfill.yml
            test -f ${src}/.github/workflows/release.yml
            test -f ${src}/.github/workflows/release-dry-run.yml
            test -f ${src}/.github/workflows/schema-drift.yml
            touch $out
          '';

          # Run deterministic offline registry-manifest builder tests from
          # filtered source; this also proves scripts/ survives the filter.
          registry-manifest-builder-tests =
            pkgs.runCommand "registry-manifest-builder-tests"
              {
                nativeBuildInputs = [ pkgs.python3 ];
              }
              ''
                export PYTHONDONTWRITEBYTECODE=1
                cd ${src}/scripts/tests
                python3 -m unittest discover -v
                touch $out
              '';
        };
      }
    );
}
