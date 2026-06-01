fn main() {
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=GIT_HASH={hash}");
            println!("cargo:rustc-env=GIT_HASH_AVAILABLE=1");
        }
    }
}
