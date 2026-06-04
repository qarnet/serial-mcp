use std::path::PathBuf;
use std::process::Command;

fn main() {
    let hooks_dir = git_hooks_dir();

    let src = repo_root().join("scripts").join("pre-push");
    let dst = hooks_dir.join("pre-push");

    std::fs::copy(&src, &dst).unwrap_or_else(|e| {
        eprintln!("failed to copy {} -> {}: {e}", src.display(), dst.display());
        std::process::exit(1);
    });

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }

    println!("installed pre-push hook");
}

fn repo_root() -> PathBuf {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .expect("git rev-parse --show-toplevel failed");
    PathBuf::from(trimmed_stdout(out))
}

fn git_hooks_dir() -> PathBuf {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .expect("git rev-parse --git-path hooks failed");
    repo_root().join(trimmed_stdout(out))
}

fn trimmed_stdout(out: std::process::Output) -> String {
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("git command failed: {stderr}");
        std::process::exit(1);
    }
    String::from_utf8(out.stdout)
        .expect("non-utf8 git output")
        .trim()
        .to_string()
}
