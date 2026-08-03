#!/bin/bash
# Install the build and runtime dependencies for Discord Recorder, then build it.
set -e

detect_os() {
    if   [[ -f /etc/arch-release ]]; then OS="arch"
    elif [[ -f /etc/debian_version ]]; then OS="debian"
    else
        echo "❌ Unsupported distribution — install the dependencies listed in README.md manually."
        exit 1
    fi
}

detect_desktop() {
    local desktop="${XDG_CURRENT_DESKTOP,,}"
    case "$desktop" in
        *kde*|*plasma*) PORTAL="kde" ;;
        *gnome*)        PORTAL="gnome" ;;
        *sway*|*wlroots*|*hyprland*) PORTAL="wlr" ;;
        *) PORTAL="" ;;
    esac
}

echo "🎥 Discord Recorder installation"
echo "================================"

detect_os
detect_desktop
SESSION="${XDG_SESSION_TYPE:-unknown}"
echo "📌 Distribution : $OS"
echo "📌 Session      : $SESSION"
echo "📌 Portal       : ${PORTAL:-unknown (install one manually for Wayland capture)}"

# ---------------------------------------------------------------------
# Dependencies
#
# ffmpeg is a *runtime* dependency: the recorder drives the ffmpeg CLI.
# The -dev packages are only needed to compile (alsa/cpal, xcb/screenshots,
# wayland/libwayshot, pipewire for the Wayland capture, clang for the webcam
# bindings).
# ---------------------------------------------------------------------
echo ""
echo "📦 Installing dependencies..."

if [[ "$OS" == "debian" ]]; then
    sudo apt update
    sudo apt install -y \
        build-essential clang pkgconf nasm curl \
        ffmpeg \
        libasound2-dev libpipewire-0.3-dev libclang-dev \
        libxcb1-dev libxcb-randr0-dev libxcb-render0-dev \
        libxcb-shm0-dev libxcb-xfixes0-dev \
        libwayland-dev libv4l-dev

    case "$PORTAL" in
        kde)   sudo apt install -y xdg-desktop-portal-kde ;;
        gnome) sudo apt install -y xdg-desktop-portal-gnome ;;
        wlr)   sudo apt install -y xdg-desktop-portal-wlr ;;
        *)     echo "⚠️  Unknown desktop: install the matching xdg-desktop-portal backend yourself." ;;
    esac
elif [[ "$OS" == "arch" ]]; then
    sudo pacman -Syu --needed --noconfirm \
        base-devel clang pkgconf nasm \
        ffmpeg alsa-lib libpipewire libxcb wayland v4l-utils

    case "$PORTAL" in
        kde)   sudo pacman -S --needed --noconfirm xdg-desktop-portal-kde ;;
        gnome) sudo pacman -S --needed --noconfirm xdg-desktop-portal-gnome ;;
        wlr)   sudo pacman -S --needed --noconfirm xdg-desktop-portal-wlr ;;
        *)     echo "⚠️  Unknown desktop: install the matching xdg-desktop-portal backend yourself." ;;
    esac
fi

# ---------------------------------------------------------------------
# Sanity checks
# ---------------------------------------------------------------------
echo ""
echo "🔍 Verifying the toolchain..."
command -v cargo >/dev/null || {
    echo "❌ Rust is not installed. See https://rustup.rs"
    exit 1
}
command -v ffmpeg >/dev/null || {
    echo "❌ ffmpeg is still missing — the recorder cannot encode without it."
    exit 1
}
echo "✅ $(ffmpeg -version | head -1)"

if [[ "$SESSION" == "wayland" ]] && ! busctl --user introspect org.freedesktop.portal.Desktop \
        /org/freedesktop/portal/desktop 2>/dev/null | grep -q ScreenCast; then
    echo "⚠️  No ScreenCast portal is answering on the session bus."
    echo "    Wayland desktop capture will not work until a portal backend is running."
fi

# ---------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------
echo ""
echo "🔧 Building (release)..."
cargo build --release

# ---------------------------------------------------------------------
# Desktop entry
# ---------------------------------------------------------------------
mkdir -p "$HOME/Videos/discord-recordings" "$HOME/.local/share/applications"
DESKTOP_ENTRY="$HOME/.local/share/applications/discord-recorder.desktop"
cat > "$DESKTOP_ENTRY" <<EOF
[Desktop Entry]
Name=Discord Recorder
Comment=Screen recording with a Discord-inspired interface
Exec=$PWD/target/release/discord-recorder
Icon=applications-multimedia
Terminal=false
Type=Application
Categories=AudioVideo;Recorder;
Keywords=screen;record;video;audio;webcam;
EOF

echo ""
echo "🎉 Done."
echo "🚀 Run:        ./target/release/discord-recorder"
echo "📁 Recordings: $HOME/Videos/discord-recordings/"
echo "⚙️  Config:     $HOME/.config/discord-recorder/config.json"
