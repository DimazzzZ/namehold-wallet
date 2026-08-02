use std::path::Path;

fn main() {
    // Tauri's externalBin bundling validates that each sidecar path exists
    // (with a `-<target-triple>` suffix) BEFORE this build proceeds. The
    // namehold-syncd daemon lives in this same Cargo package, so building it
    // triggers this same build.rs — a chicken-and-egg problem: we can't build
    // the daemon because the daemon's own build wants the daemon binary staged.
    //
    // Break the cycle by creating an empty placeholder for the *host* target
    // triple when one doesn't already exist. The real binary is copied over it
    // by `build-sidecar.sh` (dev) or the release workflow (CI). An empty stub
    // is enough to satisfy Tauri's existence check for `cargo build`/`tauri dev`;
    // production bundling always overwrites it with the compiled daemon first.
    ensure_sidecar_placeholder();

    tauri_build::build()
}

/// Create an empty `binaries/namehold-syncd-<host-triple>[.exe]` if it's missing,
/// so `cargo build` and `tauri dev` succeed without a pre-staged sidecar.
fn ensure_sidecar_placeholder() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.is_empty() {
        return;
    }
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let path_str = format!("binaries/namehold-syncd-{target}{ext}");
    let path = Path::new(&path_str);
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Best-effort: if this fails, the Tauri check below surfaces a clear error.
    let _ = std::fs::File::create(path);
    println!(
        "cargo:warning=created empty sidecar placeholder {path_str} (run build-sidecar.sh to stage the real daemon)"
    );
}
