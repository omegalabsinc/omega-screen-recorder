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

        // Link x264 library explicitly (ffmpeg-sys-next doesn't always link it correctly)
        // vcpkg names the library file as "libx264.lib"
        println!("cargo:rustc-link-lib=static=libx264");
    }
}
