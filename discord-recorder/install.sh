#!/bin/bash
set -e

# --- helper ----------------------------------------------------------
detect_os(){
    if   [[ -f /etc/arch-release ]]; then OS="arch"
    elif [[ -f /etc/debian_version ]]; then OS="debian"
    else
        echo "❌ Unsupported distribution – exiting."
        exit 1
    fi
}
detect_display(){
    if   [[ -n "$WAYLAND_DISPLAY" ]]; then DISP="wayland"
    elif [[ -n "$DISPLAY" ]];          then DISP="x11"
    else
        echo "⚠️  Could not auto-detect display server – assuming X11."
        DISP="x11"
    fi
}

# --- banner ----------------------------------------------------------
echo "🎥 Discord Recorder Installation Script"
echo "======================================"

# ---------------------------------------------------------------------
# 1. detect environment
# ---------------------------------------------------------------------
detect_os
detect_display
echo "📌 Detected OS : $OS"
echo "📌 Display      : $DISP"

# ---------------------------------------------------------------------
# 2. clone + build
# ---------------------------------------------------------------------
echo ""
echo "⬇️  Cloning Discord Recorder (Octocord)..."
[[ -d discord-recorder ]] || git clone https://github.com/s-b-repo/octocord.git discord-recorder
cd discord-recorder

echo "🔧 Building project..."
if ! cargo build --release; then
    echo "⚠️  Build failed – retrying with RUST_BACKTRACE=1..."
    export RUST_BACKTRACE=1
    cargo clean
    cargo build --release || {
        echo "❌ Build failed again – aborting."
        exit 1
    }
fi

# ---------------------------------------------------------------------
# 3. choose audio stack
# ---------------------------------------------------------------------
echo ""
echo "🔊 Choose your audio system:"
echo "  1) PulseAudio (default, stable)"
echo "  2) PipeWire   (modern replacement)"
read -rp "Select option [1-2]: " AUDIO_CHOICE
case "$AUDIO_CHOICE" in
    2) AUDIO="pipewire" ;;
    *) AUDIO="pulseaudio" ;;
esac
echo "✅ Using audio system: $AUDIO"

# ---------------------------------------------------------------------
# 4. install dependencies
# ---------------------------------------------------------------------
echo ""
echo "📦 Installing system dependencies..."

if [[ "$OS" == "arch" ]]; then
    sudo pacman -Syu --needed --noconfirm \
        base-devel clang pkg-config \
        pango cairo gdk-pixbuf2 gtk3 \
        v4l-utils ffmpeg alsa-lib libv4l \
        x265 libvpx libva libvdpau

    # audio
    if [[ "$AUDIO" == "pulseaudio" ]]; then
        if pacman -Qi pipewire-pulse &>/dev/null; then
            echo "⚠️  Removing pipewire-pulse to avoid conflicts..."
            sudo pacman -Rns --noconfirm pipewire-pulse
        fi
        sudo pacman -S --needed --noconfirm pulseaudio libpulse
    else
        if pacman -Qi pulseaudio &>/dev/null; then
            echo "⚠️  Removing pulseaudio to switch to PipeWire..."
            sudo pacman -Rns --noconfirm pulseaudio
        fi
        sudo pacman -S --needed --noconfirm \
            pipewire pipewire-alsa pipewire-pulse wireplumber
    fi

    # display
    if [[ "$DISP" == "wayland" ]]; then
        sudo pacman -S --needed --noconfirm wayland wayland-protocols libwayland
    else
        sudo pacman -S --needed --noconfirm \
            xorg-server xorg-xwininfo libx11 libxcb
    fi

    # FFmpeg headers fix
    if [[ ! -f /usr/include/libavcodec/avfft.h && -d /usr/include/ffmpeg ]]; then
        echo "🧩 Creating FFmpeg header compatibility links..."
        for d in libavcodec libavformat libavutil libavfilter libavdevice libswresample libswscale; do
            [[ -d "/usr/include/ffmpeg/$d" && ! -e "/usr/include/$d" ]] && \
                sudo ln -sfT "ffmpeg/$d" "/usr/include/$d"
        done
    fi

elif [[ "$OS" == "debian" ]]; then
    sudo apt update
    sudo apt install -y \
        build-essential clang pkg-config \
        libxcb1-dev libxcb-shm0-dev libxcb-xfixes0-dev \
        libxcb-randr0-dev libxcb-xinerama0-dev \
        libxcb-xtest0-dev libxcb-shape0-dev libxcb-xkb-dev \
        libxcb-image0-dev libxcb-icccm4-dev libxcb-render-util0-dev \
        libxcb-util0-dev libxcb-render0-dev \
        libxkbcommon-dev libxkbcommon-x11-dev \
        libpango1.0-dev libasound2-dev libv4l-dev \
        libavcodec-dev libavformat-dev libswscale-dev libavutil-dev \
        ffmpeg curl

    # audio
    if [[ "$AUDIO" == "pulseaudio" ]]; then
        sudo apt install -y pulseaudio libpulse-dev
    else
        sudo apt install -y pipewire pipewire-pulse libpipewire-0.3-dev
    fi

    # display
    if [[ "$DISP" == "wayland" ]]; then
        sudo apt install -y libwayland-dev wayland-protocols
    else
        sudo apt install -y libx11-dev libx11-xcb-dev
    fi
fi

# ---------------------------------------------------------------------
# 5. verify FFmpeg libraries
# ---------------------------------------------------------------------
echo ""
echo "🔍 Verifying FFmpeg shared libraries..."
CHECKSUM_FILE="/tmp/ffmpeg_checksums.txt"
find /usr/lib* -type f -name "libav*.so*" -exec sha256sum {} \; > "$CHECKSUM_FILE"
if [[ -s "$CHECKSUM_FILE" ]]; then
    echo "✅ FFmpeg libraries found."
else
    echo "❌ FFmpeg shared libraries missing – please reinstall ffmpeg."
    exit 1
fi

# ---------------------------------------------------------------------
# 6. desktop entry + directories
# ---------------------------------------------------------------------
echo ""
echo "📁 Setting up directories..."
mkdir -p "$HOME/Videos/discord-recordings"
mkdir -p "$HOME/.local/share/applications"

DESKTOP_ENTRY="$HOME/.local/share/applications/discord-recorder.desktop"
cat > "$DESKTOP_ENTRY" <<EOF
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

# ---------------------------------------------------------------------
# 7. finish
# ---------------------------------------------------------------------
echo ""
echo "🎉 Installation complete!"
echo ""
echo "🚀 Run:            ./target/release/discord-recorder"
echo "📁 Recordings:     $HOME/Videos/discord-recordings/"
echo "⚙️  Config file:    $HOME/.config/discord-recorder/config.json"
echo "🧮 FFmpeg checksum: $CHECKSUM_FILE"
echo "🎨 Desktop entry:   $DESKTOP_ENTRY"
echo ""
echo "You can now launch it from your application menu!"
