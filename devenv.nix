{ pkgs, lib, ... }:

let
  # Libraries gpui links against or opens at runtime (via dlopen).
  runtimeLibs = with pkgs; [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
    fontconfig
    freetype
    # X11 fallback, only used when there is no Wayland session
    libxcb
    libx11
    libxcursor
    libxi
    libxrandr
  ];
in
{
  # GPUI Kit 0.7.3 and its selected dependency graph require Rust 1.92+.
  # Use the latest stable, matching flake.nix's dev shell.
  languages.rust = {
    enable = true;
    channel = "stable";
    components = [
      "rustc"
      "cargo"
      "clippy"
      "rustfmt"
      "rust-analyzer"
      "rust-src"
    ];
    # mold is on by default on x86_64 Linux and makes the big gpui link much faster.
    # If you ever hit a linker problem, set this to false.
    # mold.enable = false;
  };

  packages = [
    pkgs.pkg-config
    pkgs.cmake
    pkgs.cargo-audit
  ] ++ runtimeLibs;

  # Without this, gpui cannot find libwayland / libvulkan at runtime on NixOS.
  env.LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;

  enterShell = ''
    echo "yomu dev shell: $(rustc --version)"
    echo "  cargo run --release   build and start the reader"
    echo "  devenv test           run the unit and UI tests"
  '';

  # `devenv test`
  enterTest = ''
    cargo test
  '';
}
