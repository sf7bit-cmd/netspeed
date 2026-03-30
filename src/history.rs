name: Build Windows EXE

on:
  push:
    branches: [ main ]
  workflow_dispatch:   # 手動実行ボタンも有効化

jobs:
  build:
    runs-on: windows-latest

    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-pc-windows-msvc

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: Build release
        run: cargo build --release --target x86_64-pc-windows-msvc

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: netspeed-windows
          path: target/x86_64-pc-windows-msvc/release/netspeed.exe

      # タグpush時は GitHub Release にも自動アップロード
      - name: Create Release
        if: startsWith(github.ref, 'refs/tags/')
        uses: softprops/action-gh-release@v2
        with:
          files: target/x86_64-pc-windows-msvc/release/netspeed.exe
