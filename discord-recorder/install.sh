#!/bin/bash

set -e

echo "🎥 Discord Recorder Installation Script"
echo "======================================"

# Detect OS
if [[ -f /etc/arch-release ]]; then
    OS="arch"
elif [[ -f /etc/debian_version ]]; then
    OS="debian"
else
    echo "❌ Unsupported Linux distribution"
    echo "Currently supports Arch Linux and Debian-based systems"
    exit 1
fi

echo "📋 Installing system dependencies..."

# Install dependencies based on OS
if [[ "$OS" == "arch" ]]; then
    sudo pacman -S --needed base-devel pkg-config \
        libx11 libwayland alsa-lib ffmpeg libv4l \
        rust cargo
elif [[ "$OS" == "debian" ]]; then
    sudo apt update
    sudo apt install -y build-essential pkg-config \
        libx11-dev libwayland-dev libasound2-dev \
        libavcodec-dev libavformat-dev libavutil-dev \
        libswscale-dev libv4l-dev \
        curl
    
    # Install Rust if not present
    if ! command -v cargo &> /dev/null; then
        echo "🦀 Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
fi

echo "🔧 Building Discord Recorder..."

# Build the project
cargo build --release

echo "📁 Creating directories..."

# Create output directory
mkdir -p "$HOME/Videos/discord-recordings"

# Create desktop entry
DESKTOP_ENTRY="$HOME/.local/share/applications/discord-recorder.desktop"
cat > "$DESKTOP_ENTRY" << EOF
[Desktop Entry]
Name=Discord Recorder
Comment=High-performance screen recording with Discord UI
Exec=$PWD/target/release/discord-recorder
Icon=$PWD/assets/icon.png
Terminal=false
Type=Application
Categories=AudioVideo;Recorder;
Keywords=screen;record;video;audio;webcam;
EOF

echo "🎉 Installation complete!"
echo ""
echo "🚀 Usage:"
echo "  ./target/release/discord-recorder"
echo ""
echo "📁 Recordings will be saved to:"
echo "  $HOME/Videos/discord-recordings/"
echo ""
echo "⚙️  Configuration file:"
echo "  $HOME/.config/discord-recorder/config.json"
echo ""
echo "🎨 Launch from your application menu or run the command above!"