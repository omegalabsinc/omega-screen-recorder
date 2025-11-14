use std::env;

fn main() {
    // Tell cargo to rerun this script if these environment variables change
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    // On macOS, help find FFmpeg libraries
    if cfg!(target_os = "macos") {
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

        // For static linking, we need to link additional system frameworks
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=Security");
    }
}
