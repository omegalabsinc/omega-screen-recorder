fn main() {
    // Link DirectShow libraries on Windows for FFmpeg
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=strmiids");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=oleaut32");
        println!("cargo:rustc-link-lib=uuid");
        println!("cargo:rustc-link-lib=mfplat");
        println!("cargo:rustc-link-lib=mfuuid");

        // Link x264 library for FFmpeg (required for libx264 encoder)
        println!("cargo:rustc-link-lib=static=x264");
    }
}
