// Embed the app icon in the Windows executable (shown in Explorer, the taskbar and Alt-Tab),
// and the build number shown in About.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    embed_build_number();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Secrets");
        res.set("FileDescription", "Secrets: AWS Secrets Manager");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}

/// SECRETS_BUILD: the abbreviated git commit hash shown in About, with "+dirty" when there
/// are uncommitted changes; "dev" outside a git checkout.
fn embed_build_number() {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let build = match git(&["rev-parse", "--short", "HEAD"]).filter(|h| !h.is_empty()) {
        Some(hash) => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if dirty { format!("{hash}+dirty") } else { hash }
        }
        None => "dev".to_string(),
    };
    println!("cargo:rustc-env=SECRETS_BUILD={build}");
    // Re-run after commits and checkouts. These are watched even when they don't exist yet:
    // Cargo then re-runs this script on every build, so a new repository is picked up.
    for f in [".git/HEAD", ".git/logs/HEAD", ".git/index"] {
        println!("cargo:rerun-if-changed={f}");
    }
}
