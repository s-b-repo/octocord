# Discord Recorder (Octocord)

A screen recorder with a Discord-inspired UI, written in Rust. Captures the desktop,
system audio, the microphone and a webcam overlay, and encodes with ffmpeg — on the GPU
when one is available.

## Features

- 🖥️ **Wayland and X11** — Wayland desktops are captured through the
  `xdg-desktop-portal` ScreenCast API and PipeWire; X11 sessions use `x11grab`
- 🎞️ **Any resolution** — records at the monitor's native size (any size, including
  HiDPI and odd dimensions), or downscales to 720p/1080p/1440p/2160p or a custom
  `W×H`, always preserving aspect ratio and never upscaling
- 🎚️ **Four video and four audio quality tiers**, independent of the resolution
- 🔊 **System audio, microphone, or both mixed** — "system audio" is the monitor of
  your default output, which is where desktop sound actually lives
- 📹 **Webcam overlay** composited into the recording by ffmpeg
- ⚡ **Hardware encoding** via VAAPI (`h264_vaapi`) with an automatic `libx264` fallback
- 🎨 **Discord theming** in Dark, Light and AMOLED

## Requirements

- Rust 1.88 or newer
- **ffmpeg** — the recorder shells out to it; nothing works without it
- A Wayland compositor with a working screencast portal (KDE, GNOME, wlroots), or X11

### Debian / Ubuntu / Kali

```bash
sudo apt install -y \
    pkgconf ffmpeg libasound2-dev libpipewire-0.3-dev libclang-dev \
    libxcb1-dev libxcb-randr0-dev libxcb-render0-dev libxcb-shm0-dev \
    libxcb-xfixes0-dev libwayland-dev

# for the Wayland capture path, install the portal backend for your desktop
sudo apt install -y xdg-desktop-portal-kde     # KDE Plasma
# sudo apt install -y xdg-desktop-portal-gnome # GNOME
# sudo apt install -y xdg-desktop-portal-wlr   # sway / wlroots
```

`libclang-dev` and `nasm` are only needed for the optional `webcam` feature (it builds
nokhwa, which compiles mozjpeg). Build with `--no-default-features` to skip both.

### Arch Linux

```bash
sudo pacman -S --needed base-devel clang pkgconf ffmpeg alsa-lib libpipewire \
    libxcb wayland v4l-utils
sudo pacman -S --needed xdg-desktop-portal-kde   # or -gnome / -wlr
```

## Build and run

```bash
git clone https://github.com/s-b-repo/octocord.git
cd octocord
cargo build --release
./target/release/discord-recorder
```

Without a webcam preview (skips the nokhwa/mozjpeg build):

```bash
cargo build --release --no-default-features
```

## How capture works

| Session | Video source | Preview |
|---|---|---|
| Wayland + portal | ScreenCast portal → PipeWire → ffmpeg stdin | live, from the same stream |
| Wayland, no portal | `x11grab` (only sees X11 windows) | none |
| X11 | `x11grab` | live, via XGetImage |

On Wayland the compositor asks which screen to share the first time you record. The
portal's *restore token* is saved in `config.json`, so later recordings start without a
dialog. **Settings → Forget screen permission** clears it.

A Wayland compositor never exposes the desktop to X11, so `x11grab` on a Wayland session
records a black frame with only X11 windows visible. That is why the portal path exists
and why it is preferred automatically.

## Output

- **Container**: Matroska (`.mkv`) with H.264 video and AAC audio
- **Audio only**: FLAC (`.flac`)
- **Split output**: `<name>.video.mkv` plus `<name>.audio.flac`
- **Location**: `~/Videos/discord-recordings` by default

### Video quality

| Tier | Bitrate cap | CRF | Frame rate | x264 preset |
|---|---|---|---|---|
| Low | 1 Mbps | 28 | 30 | veryfast |
| Medium | 2.5 Mbps | 23 | 30 | veryfast |
| High | 5 Mbps | 20 | 60 | fast |
| Ultra | 10 Mbps | 18 | 60 | medium |

### Audio quality

| Tier | Sample rate | Bitrate |
|---|---|---|
| Low | 22.05 kHz | 64 kbps |
| Medium | 44.1 kHz | 128 kbps |
| High | 48 kHz | 256 kbps |
| Lossless | 96 kHz | 320 kbps |

### Resolution

`Native` keeps the captured size. Presets and custom sizes scale down only — a 1080p
monitor recorded at "2160p" stays 1080p — and every result is rounded to even
dimensions, which `yuv420p` and every hardware encoder require.

## Hotkeys

| Action | Default |
|---|---|
| Start/stop recording | Ctrl+R |
| Pause/resume | Ctrl+P |
| Toggle webcam | Ctrl+W |

All three are rebindable in Settings.

## Configuration

`~/.config/discord-recorder/config.json`, written whenever a recording starts or you
press **Save settings**. Files written by older versions keep loading; missing fields
take their defaults.

```json
{
  "output_directory": "/home/you/Videos/discord-recordings",
  "video_quality": "High",
  "audio_quality": "High",
  "output_resolution": "Native",
  "encoder": "Auto",
  "audio_source": "System",
  "capture_cursor": true,
  "discord_theme": "Dark",
  "screencast_restore_token": "..."
}
```

## Troubleshooting

**"Failed to launch ffmpeg binary"** — install ffmpeg and make sure it is on `PATH`.

**Recording is black on Wayland** — no screencast portal is running, so the recorder
fell back to `x11grab`. Install the portal backend for your desktop and check
`systemctl --user status xdg-desktop-portal`.

**The screen picker appears on every recording** — the compositor did not return a
restore token (some portal backends do not support persistence).

**No system audio** — the recorder reads the monitor of the default sink through
PulseAudio/PipeWire. With a bare ALSA setup only microphone capture is possible.

**Hardware encoding unavailable** — the recorder runs a real one-frame VAAPI encode at
startup to decide. If it fails it silently uses `libx264`; force either one in Settings.

**Webcam is busy** — the preview releases `/dev/video*` when recording starts, since
ffmpeg needs exclusive access. Another running app (a browser tab, a meeting client)
will still hold it.

## Development

```
src/
├── main.rs             # entry point
├── gui.rs              # Discord-themed egui interface
├── portal.rs           # xdg-desktop-portal ScreenCast handshake
├── pipewire_capture.rs # PipeWire stream → frames
├── video.rs            # ffmpeg command construction and process control
├── screen.rs           # X11 preview capture and display enumeration
├── audio.rs            # audio device enumeration and level metering
├── webcam.rs           # webcam preview
└── config.rs           # persisted settings
```

Two examples double as end-to-end checks:

```bash
cargo run --example screencast_probe          # portal + PipeWire, writes a PNG
cargo run --example record_probe -- 5 720 high system auto   # a real recording
```

`cargo test` covers the configuration and quality mappings.

## License

MIT — see [LICENSE](LICENSE).
