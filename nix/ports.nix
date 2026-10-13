# The two ports and the reference, built against the pinned adi-sms source `ref`. sms runs ref's src/ with
# `python`, ref's runtime venv (Python and redis); model compiles ref's spec/model/Model.hs and its build replays
# the golden vectors and checks the model's properties and mutants. sms-rs builds with nixpkgs' rustPlatform;
# its check phase replays ref's golden vectors and runs the port's properties (rust/tests/port.rs). sms-hs
# compiles ref's spec/model/Fate.hs unchanged with the model's GHC; its build runs haskell/Test.hs on every
# core (+RTS -N). sms-hs itself runs on one core unless given +RTS -N: on this handoff-bound machine, the
# cross-core wakeups cost more than the work per message.
{
  pkgs,
  lib,
  ref,
  python,
}:
let
  sms = pkgs.runCommand "sms" { meta.mainProgram = "sms"; } ''
    mkdir -p $out/bin
    printf '#!%s\nPYTHONPATH=${ref}/src exec ${python} -P "$@"\n' ${pkgs.runtimeShell} > $out/bin/python
    printf '#!%s\nexec %s -m adi_sms "$@"\n' ${pkgs.runtimeShell} $out/bin/python > $out/bin/sms
    chmod +x $out/bin/python $out/bin/sms
  '';

  # p is a package set: pkgs, or pkgsStatic (musl, static) and its aarch64 cross, which skips the tests
  rs =
    p:
    p.rustPlatform.buildRustPackage {
      pname = "sms-rs";
      version = "0.1.0";
      src = ../rust;
      cargoLock.lockFile = ../rust/Cargo.lock;
      env.ADI_SMS = "${ref}"; # tests/port.rs reads the vectors and spec.json from here
      meta.mainProgram = "sms-rs";
    };
  sms-rs = rs pkgs;

  ghc = pkgs.haskell.packages.ghc9103.ghcWithPackages (p: [
    p.QuickCheck
    p.aeson
    p.async
    p.parallel
  ]); # the GHC of adi-sms's nix/model.nix, plus async and parallel
  # link: extra GHC flags, empty for sms-hs; sms-hs-static links glibc and the RTS libraries statically
  hs =
    name: link:
    pkgs.runCommand name
      {
        src = ../haskell;
        nativeBuildInputs = [ ghc ];
        meta.mainProgram = "sms-hs";
      }
      ''
        flags="-O2 -threaded -rtsopts -Wall -Werror -i${ref}/spec/model -i$src"
        ghc $flags ${link} -main-is Sim -outputdir $TMPDIR/sim -o $TMPDIR/sms-hs $src/Sim.hs
        ghc $flags -outputdir $TMPDIR/test -o $TMPDIR/sms-hs-test $src/Test.hs
        $TMPDIR/sms-hs-test ${ref}/src/adi_sms/spec.json +RTS -N
        install -Dm755 $TMPDIR/sms-hs $out/bin/sms-hs
      '';
  sms-hs = hs "sms-hs" "";
  static = lib.optionalAttrs (pkgs.stdenv.hostPlatform.system == "x86_64-linux") {
    sms-rs-static = rs pkgs.pkgsStatic;
    sms-rs-aarch64-static = rs pkgs.pkgsCross.aarch64-multiplatform.pkgsStatic;
    # GHC's RTS links gmp, libffi, libdw (and its libelf, zlib, bzip2, xz, zstd) and libnuma: all static here
    sms-hs-static = hs "sms-hs-static" (
      "-optl-static -optl-pthread -optl-lelf -optl-lz -optl-lbz2 -optl-llzma -optl-lzstd -optl-lnuma "
      + lib.concatMapStringsSep " " (l: "-optl-L${lib.getLib l}/lib") (
        with pkgs;
        [
          glibc.static
          elfutils
          zlib.static
          (gmp.override { withStatic = true; })
          (bzip2.override { enableStatic = true; })
          (zstd.override { enableStatic = true; })
        ]
        ++
          map
            (
              l:
              l.overrideAttrs {
                dontDisableStatic = true;
                doCheck = false;
              }
            )
            [
              libffi
              xz
              numactl
            ]
      )
    );
  };
  model = pkgs.runCommand "adi-sms-model" { nativeBuildInputs = [ ghc ]; } ''
    ghc -O1 -Wall -Werror -i${ref}/spec/model -outputdir $TMPDIR/ghc -o $TMPDIR/model ${ref}/spec/model/Model.hs
    $TMPDIR/model ${ref}/src/adi_sms/spec.json ${ref}/spec/generated/vectors.json | tee $TMPDIR/log
    install -Dm644 $TMPDIR/log $out/log
  '';
in
{
  packages = {
    inherit
      sms
      model
      sms-rs
      sms-hs
      ;
  }
  // static;
  checks = { inherit sms-rs sms-hs; };
}
