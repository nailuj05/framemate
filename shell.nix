{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    # Toolchain and target come from rust-toolchain.toml; nixpkgs' rustc has host std only.
    rustup
    # Unwrapped: the nix cc-wrapper injects host glibc headers and breaks --target builds.
    llvmPackages.clang-unwrapped

    deno
    flatpak
    shellcheck
  ];
}
