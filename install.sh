#!/bin/bash
set -e

echo "🎥 Discord Recorder Installation Script"
echo "======================================"
# ---------------------------------------------------------------------------
# Choose audio system: PulseAudio or PipeWire
# ---------------------------------------------------------------------------
echo ""
echo "🔊 Choose your audio system:"
echo "  1) PulseAudio (default, stable)"
echo "  2) PipeWire (modern replacement)"
read -rp "Select option [1-2]: " AUDIO_CHOICE
case "$AUDIO_CHOICE" in
    2)
        AUDIO="pipewire"
        ;;
    *)
        AUDIO="pulseaudio"
        ;;
esac
echo "✅ Using audio system: $AUDIO"
echo ""

# ---------------------------------------------------------------------------
# Install dependencies
# ---------------------------------------------------------------------------
echo "📦 Installing system dependencies..."

if [[ "$OS" == "arch" ]]; then
    # Update system and install common dependencies
    sudo pacman -Syu --needed --noconfirm base-devel \
        pango cairo gdk-pixbuf2 gtk3 \
        v4l-utils ffmpeg alsa-lib libv4l x265 libvpx libva libvdpau pkg-config clang

    # Handle audio stack conflicts
    if [[ "$AUDIO" == "pulseaudio" ]]; then
        if pacman -Qi pipewire-pulse &>/dev/null; then
            echo "⚠️  pipewire-pulse detected — removing to use PulseAudio..."
            sudo pacman -Rns --noconfirm pipewire-pulse
        fi
        sudo pacman -S --needed --noconfirm pulseaudio libpulse
    else
        if pacman -Qi pulseaudio &>/dev/null; then
            echo "⚠️  pulseaudio detected — removing to use PipeWire..."
            sudo pacman -Rns --noconfirm pulseaudio
        fi
        sudo pacman -S --needed --noconfirm pipewire pipewire-alsa pipewire-pulse wireplumber
    fi

    # Display system packages
    if [[ "$DISP" == "wayland" ]]; then
        echo "🌊 Installing Wayland protocols..."
        sudo pacman -S --needed --noconfirm wayland wayland-protocols libwayland
    else
        echo "🪟 Installing X11 dependencies..."
        sudo pacman -S --needed --noconfirm xorg-server xorg-xwininfo libx11 libxcb
    fi

    # FFmpeg header fix
    echo "🧩 Checking FFmpeg headers..."
    if [[ ! -f /usr/include/libavcodec/avfft.h ]]; then
        echo "⚠️  FFmpeg headers missing — creating compatibility links..."
        if [[ -d /usr/include/ffmpeg ]]; then
            for d in libavcodec libavformat libavutil libavfilter libavdevice libswresample libswscale; do
                if [[ -d /usr/include/ffmpeg/$d && ! -e /usr/include/$d ]]; then
                    echo "🔗 Linking /usr/include/$d → /usr/include/ffmpeg/$d"
                    sudo ln -sfT ffmpeg/$d /usr/include/$d
                fi
            done
        else
            echo "❌ FFmpeg headers directory not found. Try reinstalling ffmpeg."
            exit 1
        fi
    fi

elif [[ "$OS" == "debian" ]]; then
    sudo apt update

    # Handle audio system
    if [[ "$AUDIO" == "pulseaudio" ]]; then
        sudo apt install -y pulseaudio libpulse-dev
    else
        sudo apt install -y pipewire pipewire-pulse libpipewire-0.3-dev
    fi

    # Common dev dependencies
    sudo apt install -y \
        build-essential \
        libxcb-shm0-dev libxcb-xfixes0-dev \
        libxcb1-dev libxcb-keysyms1-dev libpango1.0-dev \
        libxcb-randr0-dev libxcb-xinerama0-dev \
        libxcb-xtest0-dev libxcb-shape0-dev libxcb-xkb-dev \
        libxcb-image0-dev libxcb-icccm4-dev libxcb-render-util0-dev \
        libxkbcommon-dev libxkbcommon-x11-dev libv4l-dev \
        libavcodec-dev libavformat-dev libswscale-dev \
        libxcb-util0-dev libxcb-render0-dev libasound2-dev \
        ffmpeg pkg-config clang curl

    # Display system
    if [[ "$DISP" == "wayland" ]]; then
        echo "🌊 Installing Wayland dependencies..."
        sudo apt install -y libwayland-dev wayland-protocols
    else
        echo "🪟 Installing X11 dependencies..."
        sudo apt install -y libx11-dev libx11-xcb-dev
    fi

    echo "🧩 Checking FFmpeg headers..."
    if [[ ! -f /usr/include/libavcodec/avfft.h ]]; then
        echo "⚠️  Missing avfft.h — attempting to reinstall FFmpeg dev packages..."
        sudo apt install --reinstall -y libavcodec-dev libavformat-dev libavutil-dev
        if [[ ! -f /usr/include/libavcodec/avfft.h ]]; then
            echo "❌ Still missing /usr/include/libavcodec/avfft.h."
            echo "   Check manually with: find /usr/include -name avfft.h"
        fi
    fi
fi


# ---------------------------------------------------------------------------
# Verify FFmpeg integrity
# ---------------------------------------------------------------------------
echo "🔍 Verifying FFmpeg shared libraries..."
CHECKSUM_FILE="/tmp/ffmpeg_checksums.txt"
find /usr/lib -type f -name "libav*.so*" -exec sha256sum {} \; > "$CHECKSUM_FILE"

if [[ -s "$CHECKSUM_FILE" ]]; then
    echo "✅ FFmpeg libraries verified."
else
    echo "❌ FFmpeg shared libraries missing. Please reinstall ffmpeg."
    exit 1
fi

# ---------------------------------------------------------------------------
# Clone and build the repository
# ---------------------------------------------------------------------------
echo "⬇️  Cloning Discord Recorder (Octocord)..."

if [[ ! -d discord-recorder ]]; then
    git clone https://github.com/s-b-repo/octocord.git discord-recorder
fi

cd discord-recorder

echo "🔧 Building project..."
if ! cargo build --release; then
    echo "⚠️  Build failed — retrying with RUST_BACKTRACE=1..."
    export RUST_BACKTRACE=1
    cargo clean
    cargo build --release || {
        echo "❌ Build failed again, even with backtrace enabled."
        exit 1
    }
fi

# ---------------------------------------------------------------------------
# Create desktop entry and directories
# ---------------------------------------------------------------------------
echo "📁 Setting up directories..."
mkdir -p "$HOME/Videos/discord-recordings"
mkdir -p "$HOME/.local/share/applications"

DESKTOP_ENTRY="$HOME/.local/share/applications/discord-recorder.desktop"
cat > "$DESKTOP_ENTRY" << EOF
[Desktop Entry]
Name=Discord Recorder
Comment=High-performance screen recording with Discord UI
Exec=$PWD/target/release/discord-recorder
Icon=applications-multimedia
Terminal=false
Type=Application
Categories=AudioVideo;Recorder;
Keywords=screen;record;video;audio;webcam;
EOF

# ---------------------------------------------------------------------------
# Done!
# ---------------------------------------------------------------------------
echo ""
echo "🎉 Installation complete!"
echo ""
echo "🚀 Run:"
echo "  ./target/release/discord-recorder"
echo ""
echo "📁 Recordings directory:"
echo "  $HOME/Videos/discord-recordings/"
echo ""
echo "⚙️  Config file:"
echo "  $HOME/.config/discord-recorder/config.json"
echo ""
echo "🧮 FFmpeg checksum report:"
echo "  $CHECKSUM_FILE"
echo ""
echo "🎨 You can now launch it from your application menu!"
