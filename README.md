# Discord Recorder

A high-performance screen recording application with Discord-inspired UI, built in Rust for Windows and Linux systems.

## Features

- 🎥 **Cross-Platform** - Works on Windows, Arch Linux, and Debian/Ubuntu
- 🎤 **Audio Recording** - System and microphone audio capture with multiple quality options
- 📹 **Webcam Support** - Movable and resizable webcam overlay with real-time preview
- 🎨 **Discord UI** - Beautiful Discord-inspired interface with dark theme
- ⚡ **High Performance** - Optimized for speed and stability using Rust
- 🛠️ **Hardware Acceleration** - Utilizes GPU acceleration when available

## System Requirements

### Windows
- Windows 10 or later (64-bit)
- [Rust](https://www.rust-lang.org/tools/install) 1.88.0 or later
- [FFmpeg](https://ffmpeg.org/download.html) (add to PATH)
- [Visual C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (for building)

### Arch Linux
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install system dependencies
sudo pacman -S --needed base-devel \
    xorg-server xorg-xwininfo \
    libx11 libxcb pango cairo gdk-pixbuf2 gtk3 \
    v4l-utils ffmpeg alsa-lib pulseaudio \
    libv4l libx264 libx265 libvpx libva libvdpau libpulse
```

### Debian/Ubuntu
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install system dependencies
sudo apt update
sudo apt install -y \
    build-essential \
    libx11-dev libxcb-shm0-dev libxcb-xfixes0-dev \
    libxcb1-dev libxcb-keysyms1-dev libpango1.0-dev \
    libx11-xcb-dev libxcb-randr0-dev libxcb-xinerama0-dev \
    libxcb-xtest0-dev libxcb-shape0-dev libxcb-xkb-dev \
    libxcb-image0-dev libxcb-icccm4-dev libxcb-render-util0-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libv4l-dev \
    libavcodec-dev libavformat-dev libswscale-dev \
    libxcb-util0-dev libxcb-render0-dev libasound2-dev \
    libpulse-dev ffmpeg
```

## Installation

1. **Clone the repository:**
```bash
git clone https://github.com/s-b-repo/octocord.git
cd discord-recorder
```

2. **Build the project:**
```bash
# For your current platform
cargo build --release

# Or build for a specific target
# Windows:
# cargo build --release --target x86_64-pc-windows-msvc
# Linux (x86_64):
# cargo build --release --target x86_64-unknown-linux-gnu
# Linux (ARM64):
# cargo build --release --target aarch64-unknown-linux-gnu
```

3. **Run the application:**
```bash
# Windows
.\target\release\discord-recorder.exe

# Linux
./target/release/discord-recorder
```

## Usage

1. **Launch the application** - The Discord-themed interface will appear
2. **Configure settings** - Select screen, audio device, and webcam options
3. **Choose quality** - Set video and audio quality preferences
4. **Start recording** - Click the record button or use hotkeys
5. **Stop recording** - Click stop or use hotkeys to save your recording

### Recording Controls

- **Start/Stop Recording**: Main record button in the top panel or Ctrl+R
- **Pause/Resume**: Pause button or Ctrl+P
- **Webcam Toggle**: Camera button or Ctrl+W
- **Settings**: Gear icon in the top-right corner
- **Webcam Position**: Drag and resize webcam overlay during recording

### Output Settings

- **Video Quality**: 
  - Low (720p, 30fps)
  - Medium (1080p, 30fps)
  - High (1440p, 60fps)
  - Ultra (4K, 60fps)
- **Audio Quality**: 
  - Low (22kHz, 64kbps)
  - Medium (44kHz, 128kbps)
  - High (48kHz, 192kbps)
  - Lossless (96kHz, 320kbps)
- **Format**: MP4 (H.264 video + AAC audio)
- **Default Location**:
  - Windows: `%USERPROFILE%\Videos\Discord Recordings`
  - Linux: `~/Videos/discord-recordings/`

## Configuration

The application stores configuration in:
- **Windows**: `%APPDATA%\discord-recorder\config.json`
- **Linux**: `~/.config/discord-recorder/config.json`

### Configuration Options

```json
{
  "recording": {
    "video_quality": "high",
    "audio_quality": "medium",
    "output_dir": "~/Videos/discord-recordings",
    "fps": 60,
    "audio_device": "default",
    "webcam_device": "/dev/video0"
  },
  "hotkeys": {
    "start_stop": "Ctrl+R",
    "pause_resume": "Ctrl+P",
    "toggle_webcam": "Ctrl+W"
  },
  "ui": {
    "theme": "dark",
    "window_size": [1200, 800],
    "show_fps": true
  }
}
```

## Building from Source

### Prerequisites

1. Install Rust 1.88.0 or later
2. Install platform-specific dependencies (see above)
3. Clone the repository

### Build Commands

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Cross-compile for Windows from Linux
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc

# Cross-compile for Linux ARM64 from x86_64
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

## Troubleshooting

### Common Issues

1. **No screens detected**
   - On Linux: Ensure X11 or Wayland is running and DISPLAY is set
   - On Windows: Check display drivers and permissions

2. **Audio not recording**
   - Check system audio input settings
   - Verify the correct audio device is selected in settings
   - On Linux, ensure PulseAudio is running

3. **Webcam not working**
   - Check if the webcam is detected by the system
   - Verify correct permissions (on Linux, ensure user is in the `video` group)
   - Try a different webcam device path if applicable

4. **FFmpeg errors**
   - Ensure FFmpeg is installed and in PATH
   - Check for codec support in your FFmpeg build
   - Try reinstalling FFmpeg with additional codec support

## Contributing

Contributions are welcome! Please follow these steps:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
   - Ensure X11 or Wayland is running
   - Check DISPLAY or WAYLAND_DISPLAY environment variables

2. **Audio not recording**
   - Verify audio device permissions
   - Check if PulseAudio/PipeWire is running

3. **Webcam not working**
   - Ensure camera permissions are granted
   - Check if camera is being used by another application

4. **Build errors**
   - Install all required system dependencies
   - Update Rust toolchain: `rustup update`

### Debug Mode

Run with debug logging:
```bash
RUST_LOG=debug ./target/release/discord-recorder
```

## Development

### Project Structure

```
src/
├── main.rs          # Application entry point
├── gui.rs           # Discord-themed user interface
├── screen.rs        # Screen capture implementation
├── audio.rs         # Audio recording
├── video.rs         # Video encoding
├── webcam.rs        # Webcam capture
└── config.rs        # Configuration management
```

### Adding Features

1. Fork the repository
2. Create a feature branch
3. Implement your changes
4. Add tests if applicable
5. Submit a pull request

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Acknowledgments

- Inspired by OBS Studio and Discord's design language
- Built with the Rust ecosystem and open-source libraries
- Thanks to the contributors of egui, FFmpeg, and other dependencies

## Support

For issues and feature requests, please open a GitHub issue.
For questions and discussions, use the GitHub Discussions tab.
