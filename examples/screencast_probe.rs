//! Exercises the portal + PipeWire capture path on its own.
//!
//! ```text
//! cargo run --example screencast_probe -- [restore-token]
//! ```
//!
//! Approve the compositor's screen picker when it appears. The probe grabs a few
//! frames, writes the last one to /tmp/octocord-probe.png, reports the mean
//! brightness (a black frame is exactly the failure mode x11grab has on Wayland)
//! and prints the restore token that skips the dialog next time.

use anyhow::{bail, Result};
use discord_recorder::pipewire_capture::{PipeWireCapture, PixelFormat};
use discord_recorder::portal::{self, ScreenCastOptions};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if !portal::is_available() {
        bail!("No ScreenCast portal on this session bus");
    }

    let options = ScreenCastOptions {
        capture_cursor: true,
        restore_token: std::env::args().nth(1),
    };

    println!("Requesting a screen cast session (approve the picker if it appears)...");
    let session = portal::start_screencast(&options)?;
    let stream = session.primary_stream()?;
    println!(
        "node id {}, portal size hint {:?}",
        stream.node_id, stream.size
    );

    let capture = PipeWireCapture::start(session.fd.try_clone()?, stream.node_id)?;
    let format = capture.wait_for_format(Duration::from_secs(10))?;
    println!(
        "negotiated {}x{} {:?} ({})",
        format.width,
        format.height,
        format.pixel_format,
        format.pixel_format.ffmpeg_name()
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    while capture.frames_captured() < 15 && Instant::now() < deadline {
        if let Some(err) = capture.error() {
            bail!("capture error: {}", err);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let Some(frame) = capture.latest_frame() else {
        bail!("no frames arrived in 10 s (captured {})", capture.frames_captured());
    };
    println!("captured {} frames", capture.frames_captured());

    // Mean luminance, to tell a real desktop from the black frame x11grab produces.
    let sum: u64 = frame.data.iter().map(|&b| b as u64).sum();
    let mean = sum as f64 / frame.data.len() as f64;
    println!("mean channel value: {:.2} (0 = fully black)", mean);

    let mut rgba = Vec::with_capacity(frame.data.len());
    for px in frame.data.chunks_exact(4) {
        match frame.pixel_format {
            PixelFormat::Bgrx | PixelFormat::Bgra => {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255])
            }
            PixelFormat::Rgbx | PixelFormat::Rgba => {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255])
            }
        }
    }
    let image = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| anyhow::anyhow!("frame buffer had the wrong length"))?;
    image.save("/tmp/octocord-probe.png")?;
    println!("wrote /tmp/octocord-probe.png");

    if let Some(token) = &session.restore_token {
        println!("restore token: {}", token);
    } else {
        println!("no restore token offered by this portal");
    }

    if mean < 1.0 {
        bail!("frames are black — the compositor is not delivering content");
    }
    Ok(())
}
