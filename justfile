# Falcon RGB — macOS (and portable) replacement for the MAX Falcon-20 RGB
# Windows software. `just` lists the recipes.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

app_name := "Falcon RGB"

default:
    @just --list --unsorted

# Debug build of the whole workspace
build:
    cargo build --workspace

# Fast type check without codegen — the quickest edit loop (desktop + browser build)
check:
    cargo check --workspace --all-targets
    PATH="$HOME/.cargo/bin:$PATH" cargo check -p falcon-gui --target wasm32-unknown-unknown

# Start the desktop app (debug), e.g. `just gui --simulate`
gui *args:
    cargo run -p falcon-gui -- {{args}}

# Run the CLI (debug), e.g. `just cli list` or `just cli --simulate effect wave`
cli *args:
    cargo run -p falcon-cli -- {{args}}

# Unit tests (nextest runs test binaries in parallel; falls back to cargo test)
test:
    cargo nextest run --workspace --no-tests=pass || cargo test --workspace

# Formatting + clippy, warnings are errors
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    PATH="$HOME/.cargo/bin:$PATH" cargo clippy -p falcon-gui --target wasm32-unknown-unknown -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Everything CI would run
ci: lint test

# Optimized build (thin LTO)
release:
    cargo build --workspace --release

# macOS .app bundle (ad-hoc signed) from the `dist` profile → dist/
[macos]
bundle:
    cargo build -p falcon-gui -p falcon-cli --profile dist
    scripts/bundle-mac.sh "{{app_name}}" target/dist

# One-time setup: rustup target add wasm32-unknown-unknown; brew install trunk (or cargo install
# --locked trunk). rustup's toolchain is used because Homebrew's rustc has no wasm std.

# Browser app on http://127.0.0.1:8080 (rebuilds on save); `?simulate` for the simulated keypad
web:
    PATH="$HOME/.cargo/bin:$PATH" trunk serve --config crates/falcon-gui/Trunk.toml

# Optimized browser build → dist/web/ (static files, host anywhere with HTTPS)
web-build:
    PATH="$HOME/.cargo/bin:$PATH" trunk build --release --config crates/falcon-gui/Trunk.toml

# One-time setup: rustup target add x86_64-pc-windows-msvc; cargo install --locked cargo-xwin;
# brew install llvm. Uses rustup's toolchain (Homebrew's rustc has no Windows std).

# Windows exe cross-built from macOS (static CRT via .cargo/config.toml)
[macos]
dist-windows:
    PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo xwin build -p falcon-gui -p falcon-cli --profile dist --target x86_64-pc-windows-msvc
    @echo "→ target/x86_64-pc-windows-msvc/dist/"

# sccache hit rate (build twice after `cargo clean` to see it work)
cache-stats:
    sccache --show-stats

# Remove build artifacts (sccache keeps compiled deps, so the next build is still fast)
clean:
    cargo clean
    rm -rf dist
