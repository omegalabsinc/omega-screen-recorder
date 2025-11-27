use std::env;

fn main() {
    // Tell cargo to rerun this script if these environment variables change
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    // On macOS, help find FFmpeg libraries
    #[cfg(target_os = "macos")]
    {
        // Try common installation locations
        let possible_paths = vec![
            "/opt/homebrew/lib",           // ARM Homebrew
            "/usr/local/lib",              // Intel Homebrew
            "/opt/local/lib",              // MacPorts
        ];

        for path in possible_paths {
            if std::path::Path::new(path).exists() {
                println!("cargo:rustc-link-search=native={}", path);
            }
        }

        // Check if FFMPEG_DIR is set
        if let Ok(ffmpeg_dir) = env::var("FFMPEG_DIR") {
            println!("cargo:rustc-link-search=native={}/lib", ffmpeg_dir);
        }

        // For static linking, we need to link additional system frameworks and libraries
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=Security");

        // Additional frameworks needed for static FFmpeg
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=IOSurface");
        println!("cargo:rustc-link-lib=framework=OpenCL");
        println!("cargo:rustc-link-lib=framework=OpenGL");

        // System libraries needed for static linking
        println!("cargo:rustc-link-lib=dylib=iconv");
        println!("cargo:rustc-link-lib=dylib=bz2");
        println!("cargo:rustc-link-lib=dylib=z");
    }

    // Link DirectShow libraries on Windows for FFmpeg
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=strmiids");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=oleaut32");
        println!("cargo:rustc-link-lib=uuid");
        println!("cargo:rustc-link-lib=mfplat");
        println!("cargo:rustc-link-lib=mfuuid");

        // Link x264 library explicitly (ffmpeg-sys-next doesn't always link it correctly)
        // vcpkg names the library file as "libx264.lib"
        println!("cargo:rustc-link-lib=static=libx264");
    }
}
