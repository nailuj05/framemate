{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    # Toolchain and target come from rust-toolchain.toml; nixpkgs' rustc has host std only.
    rustup
    # nix cc-wrapper injects host glibc headers and breaks --target builds.
    llvmPackages.clang-unwrapped
    pkg-config

    deno
    flatpak
    shellcheck
  ];

  # Tauri desktop building for verification
  buildInputs = with pkgs; [
    glib
    gtk3
    libsoup_3
    webkitgtk_4_1
    openssl
  ];

  CC_aarch64_unknown_linux_musl = "clang";
}
