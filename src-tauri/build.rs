fn main() {
    // Allow build to continue even if tauri-build encounters issues
    // (e.g., missing/invalid icon files on Windows)
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new()) {
        println!("cargo:warning=tauri-build warning (non-fatal): {}", e);
    }
}
