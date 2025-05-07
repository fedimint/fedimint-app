{
  inputs = {
    fedimint.url = "github:fedimint/fedimint?rev=b983d25d4c3cce1751c54e3ad0230fc507e3aeec";
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixgl.url = "github:guibou/nixGL";
    android.url = "github:tadfisher/android-nixpkgs";
  };

  outputs = { self, fedimint, flake-utils, nixpkgs, nixgl, android, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        inherit (nixpkgs) lib;
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };
        androidPkgs = {
          android-sdk = android.sdk.${system} (sdkPkgs: with sdkPkgs; [
            # Useful packages for building and testing.
            build-tools-34-0-0
            cmdline-tools-latest
            emulator
            platform-tools
            platforms-android-34

            # Other useful packages for a development environment.
            #ndk-26-1-10909125
            ndk-27-0-12077973
            # skiaparser-3
            # sources-android-34
          ]
          ++ lib.optionals (system == "aarch64-darwin") [
            # system-images-android-34-google-apis-arm64-v8a
            # system-images-android-34-google-apis-playstore-arm64-v8a
          ]
          ++ lib.optionals (system == "x86_64-darwin" || system == "x86_64-linux") [
            # system-images-android-34-google-apis-x86-64
            # system-images-android-34-google-apis-playstore-x86-64
          ]);
        } // lib.optionalAttrs (system == "x86_64-linux") {
          # Android Studio in nixpkgs is currently packaged for x86_64-linux only.
          android-studio = pkgs.androidStudioPackages.stable;
          # android-studio = pkgs.androidStudioPackages.beta;
          # android-studio = pkgs.androidStudioPackages.preview;
          # android-studio = pkgs.androidStudioPackage.canary;
        };
        nixglPkgs = import nixgl { inherit system; };

        # Import the `devShells` from the fedimint flake
        devShells = fedimint.devShells.${system};

        # Reproducibly install flutter_rust_bridge_codegen via Rust
        flutter_rust_bridge_codegen = pkgs.rustPlatform.buildRustPackage rec {
          name = "flutter_rust_bridge";

          src = pkgs.fetchFromGitHub {
            owner = "fzyzcjy";
            repo = name;
            rev = "v2.9.0";
            sha256 = "sha256-3Rxbzeo6ZqoNJHiR1xGR3wZ8TzUATyowizws8kbz0pM=";
          };

          cargoHash = "sha256-efMA8VJaQlqClAmjJ3zIYLUfnuj62vEIBKsz0l3CWxA=";
          
          # For some reason flutter_rust_bridge unit tests are failing
          doCheck = false;
        };
      in {
        devShells = {
          # You can expose all or specific shells from the original flake
          default = devShells.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs or [] ++ [
              pkgs.flutter
              pkgs.just
              pkgs.zlib
              flutter_rust_bridge_codegen
              pkgs.cargo-expand
              pkgs.jdk17

              androidPkgs.android-sdk
            ] ++ pkgs.lib.optionals (pkgs.stdenv.system == "x86_64-linux") [
              androidPkgs.android-studio
            ];

	    shellHook = ''
	      ${old.shellHook or ""}

              export LD_LIBRARY_PATH="${pkgs.zlib}/lib:$LD_LIBRARY_PATH"
              export NIXPKGS_ALLOW_UNFREE=1
              export ROOT="$PWD"
              export ANDROID_SDK_ROOT=${androidPkgs.android-sdk}/share/android-sdk
              export PATH=$ANDROID_SDK_ROOT/emulator:$ANDROID_SDK_ROOT/tools:$ANDROID_SDK_ROOT/tools/bin:$ANDROID_SDK_ROOT/platform-tools:$PATH
              export JAVA_HOME=${pkgs.jdk17}
	    '';
          });
        };
      }
    );
}
