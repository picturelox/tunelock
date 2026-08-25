fn main() {
    // Allow build to continue even if tauri-build encounters issues
    // (e.g., missing/invalid icon files on Windows)
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new()) {
        println!("cargo:warning=tauri-build warning (non-fatal): {}", e);
    }

    // Set TUNELOCK_GIT_SHA environment variable for compile-time access.
    // Falls back to "unknown" if git is not available or not a git repo.
    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TUNELOCK_GIT_SHA={}", git_sha);
}
