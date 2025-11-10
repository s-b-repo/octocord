use std::process::Command;
use std::env;

fn main() {
    // Check if we're on Linux
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-cfg=target_os=\"linux\"");
        
        // Check for required system dependencies
        println!("cargo:warning=Make sure you have the following dependencies installed:");
        println!("cargo:warning=  - libx11-dev (for X11 support)");
        println!("cargo:warning=  - libwayland-dev (for Wayland support)");
        println!("cargo:warning=  - libasound2-dev (for audio support)");
        println!("cargo:warning=  - libavcodec-dev, libavformat-dev, libavutil-dev (for FFmpeg)");
        println!("cargo:warning=  - libv4l-dev (for webcam support)");
        
        // Try to detect display server
        if env::var("WAYLAND_DISPLAY").is_ok() {
            println!("cargo:rustc-cfg=feature=\"wayland\"");
            println!("cargo:warning=Detected Wayland display server");
        } else if env::var("DISPLAY").is_ok() {
            println!("cargo:rustc-cfg=feature=\"x11\"");
            println!("cargo:warning=Detected X11 display server");
        }
    }
    
    // Set up FFmpeg linking
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avformat");
    println!("cargo:rustc-link-lib=avutil");
    println!("cargo:rustc-link-lib=swscale");
    
    // Re-run if environment changes
    println!("cargo:rerun-if-env-changed=DISPLAY");
    println!("cargo:rerun-if-env-changed=WAYLAND_DISPLAY");
}