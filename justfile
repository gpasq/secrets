# Run `just` to list recipes.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

windows_target := "x86_64-pc-windows-msvc"

# List recipes
default:
    @just --list

# Launch the app from source (optionally with an AWS profile)
run profile="":
    {{ if profile == "" { "" } else { "AWS_PROFILE=" + profile + " " } }}cargo run --quiet

# Debug build
build:
    cargo build

# Optimized build at target/release/secrets
release:
    cargo build --release

# Build target/Secrets.app (double-clickable macOS app)
[macos]
app: release
    rm -rf target/Secrets.app
    mkdir -p target/Secrets.app/Contents/MacOS target/Secrets.app/Contents/Resources
    cp assets/Info.plist target/Secrets.app/Contents/Info.plist
    cp assets/AppIcon.icns target/Secrets.app/Contents/Resources/AppIcon.icns
    cp target/release/secrets target/Secrets.app/Contents/MacOS/secrets
    @echo "Built target/Secrets.app"

# Install Secrets.app into ~/Applications (shows up in Launchpad/Spotlight)
[macos]
install: app
    mkdir -p ~/Applications
    rm -rf ~/Applications/Secrets.app
    cp -R target/Secrets.app ~/Applications/
    touch ~/Applications/Secrets.app
    @echo "Installed ~/Applications/Secrets.app"

# Install secrets.exe into ~/.cargo/bin
[windows]
install:
    cargo install --path . --locked

# Run the tests (set SECRETS_UI_SHOTS=<dir> to save UI screenshots)
test:
    cargo test

# Clippy with warnings as errors
lint:
    cargo clippy --all-targets -- -D warnings

# Format the code
fmt:
    cargo fmt

# Everything CI checks: formatting, lint, tests
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Cross-compile a Windows .exe from macOS (needs: brew install llvm nasm cmake; cargo install cargo-xwin)
[unix]
windows:
    rustup target add {{windows_target}}
    PATH="$(brew --prefix llvm)/bin:$PATH" cargo xwin build --release --target {{windows_target}}
    @echo "Built target/{{windows_target}}/release/secrets.exe"

# Regenerate icon PNG/.icns/.ico from assets/icon.svg (needs: cargo install resvg)
[macos]
icon:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    mkdir "$tmp/AppIcon.iconset"
    for s in 16 32 128 256 512; do
        resvg -w $s -h $s assets/icon.svg "$tmp/AppIcon.iconset/icon_${s}x${s}.png"
        resvg -w $((s*2)) -h $((s*2)) assets/icon.svg "$tmp/AppIcon.iconset/icon_${s}x${s}@2x.png"
    done
    iconutil -c icns "$tmp/AppIcon.iconset" -o assets/AppIcon.icns
    resvg -w 256 -h 256 assets/icon.svg assets/icon-256.png
    for s in 16 24 32 48 64 128; do resvg -w $s -h $s assets/icon.svg "$tmp/$s.png"; done
    python3 assets/make-ico.py assets/icon.ico "$tmp"/{16,24,32,48,64,128}.png assets/icon-256.png
    rm -rf "$tmp"
    echo "Updated assets/AppIcon.icns, assets/icon.ico, assets/icon-256.png"

# Remove build artifacts
clean:
    cargo clean
