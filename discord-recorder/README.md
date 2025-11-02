# Discord Recorder

A high-performance screen recording application with Discord-inspired UI, built in Rust for Linux systems.

## Features

- 🎥 **Screen Recording** - Native resolution screen capture with X11 and Wayland support
- 🎤 **Audio Recording** - System and microphone audio capture with multiple quality options
- 📹 **Webcam Recording** - Movable and resizable webcam overlay with real-time preview
- 🎨 **Discord UI** - Beautiful Discord-inspired interface with dark theme
- ⚡ **High Performance** - Optimized for speed and stability using Rust
- 🐧 **Linux Focused** - Built specifically for Arch and Debian-based distributions

## System Requirements

### For Arch Linux:
```bash
sudo pacman -S base-devel pkg-config
sudo pacman -S libx11 libwayland alsa-lib ffmpeg libv4l
```

### For Debian/Ubuntu:
```bash
sudo apt update
sudo apt install build-essential pkg-config
sudo apt install libx11-dev libwayland-dev libasound2-dev libavcodec-dev libavformat-dev libavutil-dev libv4l-dev
```

## Installation

1. **Clone the repository:**
```bash
git clone <repository-url>
cd discord-recorder
```

2. **Build the project:**
```bash
cargo build --release
```

3. **Run the application:**
```bash
./target/release/discord-recorder
```

## Usage

1. **Launch the application** - The Discord-themed interface will appear
2. **Configure settings** - Select screen, audio device, and webcam options
3. **Choose quality** - Set video and audio quality preferences
4. **Start recording** - Click the record button or use hotkeys
5. **Stop recording** - Click stop or use hotkeys to save your recording

### Recording Controls

- **Start/Stop Recording** - Main record button in the top panel
- **Hotkeys** - Configure in settings (Ctrl+R to start/stop)
- **Webcam Position** - Drag and resize webcam overlay during recording

### Output Settings

- **Video Quality**: Low (720p), Medium (1080p), High (1440p), Ultra (4K)
- **Audio Quality**: Low (22kHz), Medium (44kHz), High (48kHz), Lossless (96kHz)
- **Format**: MP4 video with H.264 encoding and AAC audio
- **Location**: ~/Videos/discord-recordings/

## Configuration

The application stores configuration in:
- **Linux**: `~/.config/discord-recorder/config.json`

Configuration includes:
- Default recording settings
- UI preferences
- Hotkey assignments
- Output directory

## Technical Details

### Architecture

- **Frontend**: egui framework with Discord-inspired styling
- **Screen Capture**: X11 and Wayland native APIs
- **Audio**: cpal library for cross-platform audio capture
- **Video Encoding**: FFmpeg for high-performance video encoding
- **Webcam**: nokhwa library for camera support

### Performance Optimizations

- Multi-threaded encoding
- Hardware-accelerated video encoding (when available)
- Efficient memory management with Rust's ownership system
- Minimal CPU overhead during recording

## Troubleshooting

### Common Issues

1. **No screens detected**
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