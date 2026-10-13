{
  description = "Rust and Haskell ports of adi-sms, conformance-tested against its golden vectors";

  inputs = {
    # The reference, pinned in flake.lock: its spec.json, golden vectors, Fate.hs, model and `sms` source. Fetched
    # with git over HTTPS. CI rewrites the URL to SSH and fetches with a read-only deploy key (ci.yml).
    adi-sms.url = "git+https://github.com/Jesssullivan/adi-sms?ref=main";
    nixpkgs.follows = "adi-sms/nixpkgs"; # one toolchain pin: the model's GHC and rustc come from it
  };

  outputs =
    { adi-sms, nixpkgs, ... }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      perSystem = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system} system);
      ports =
        pkgs: system:
        import ./nix/ports.nix {
          inherit pkgs lib;
          ref = adi-sms;
          python = "${adi-sms.packages.${system}.default}/bin/python"; # the core's runtime venv: Python and redis
        };
    in
    {
      # sms and model run the pinned adi-sms's Python reference and Haskell model; sms-rs and sms-hs are the ports.
      # chart-py draws bench/chart.svg (matplotlib). On x86_64-linux, sms-rs-static, sms-rs-aarch64-static and sms-hs-static are the release binaries.
      packages = perSystem (
        pkgs: system:
        (ports pkgs system).packages // { chart-py = pkgs.python3.withPackages (p: [ p.matplotlib ]); }
      );

      checks = perSystem (pkgs: system: (ports pkgs system).checks);

      devShells = perSystem (
        pkgs: _: {
          default = pkgs.mkShell {
            # What the recipes and CI call, nothing else: `just`; gitMinimal for public-safe and bench;
            # python3Minimal and gitleaks for public-safe; jq and valkey for conform; hyperfine for bench; the Rust toolchain for test
            # and lint; nixfmt for lint; python3Minimal, nodejs-slim and esbuild for textbook. sms-hs, sms and the model build through nix, so GHC is not here.
            packages = with pkgs; [
              just
              gitMinimal
              jq
              hyperfine
              python3Minimal
              gitleaks
              valkey
              cargo
              rustc
              rustfmt
              clippy
              nixfmt
              nodejs-slim # runs textbook/tinymt.ts with type stripping
              esbuild # textbook/tinymt.ts -> tinymt.js for index.html
            ];
            # The pinned source, which `cargo test`, `just conform` and `just public-safe` read.
            env.ADI_SMS = "${adi-sms}";
            shellHook = ''echo "adi-sms-ports shell: run \`just\` for every task"'';
          };
        }
      );
    };
}
