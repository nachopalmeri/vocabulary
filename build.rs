//! Build script: embeds the application manifest (per-monitor-v2 DPI awareness,
//! `asInvoker` privileges, Windows 10/11 compatibility) into the executable.
//!
//! We use the MSVC linker's documented `/MANIFEST:EMBED` + `/MANIFESTINPUT:` options
//! instead of pulling in an extra resource-compiler dependency.

fn main() {
    println!("cargo:rerun-if-changed=worddock.manifest");

    #[cfg(target_os = "windows")]
    {
        // Only the MSVC linker understands these flags; on other toolchains the app
        // simply builds without the manifest (it is a Windows-only application).
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            let manifest =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("worddock.manifest");
            let manifest = manifest.display().to_string();
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            if manifest.contains(' ') {
                println!("cargo:rustc-link-arg=\"/MANIFESTINPUT:{manifest}\"");
            } else {
                println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest}");
            }
        }
    }
}
