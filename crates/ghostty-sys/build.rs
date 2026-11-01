fn main() {
    println!("cargo:rerun-if-env-changed=GHOSTTY_DIR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_LIB_DIR");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rerun-if-changed=src/bridge.h");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::env::var_os("GHOSTTY_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".deps/ghostty-f64f4aca2c29b554d111b36c3d946a9bddd159ff"));
    assert!(
        source.join("include/ghostty/vt.h").exists(),
        "Run scripts/bootstrap.ps1 (Windows) or bash scripts/bootstrap.sh (macOS/Linux) first: missing Ghostty headers."
    );
    let target = std::env::var("TARGET").expect("Cargo target");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let library = if windows {
        "ghostty-vt-static"
    } else {
        "ghostty-vt"
    };
    let filename = if windows {
        "ghostty-vt-static.lib"
    } else {
        "libghostty-vt.a"
    };
    let lib_dir = std::env::var_os("GHOSTTY_LIB_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let directory = source.join("zig-out").join(&target).join("lib");
            // Compatibility with existing Windows bootstraps, for the native host only.
            if !directory.join(filename).exists()
                && windows
                && std::env::var("HOST").as_deref() == Ok(&target)
            {
                source.join("zig-out/lib")
            } else {
                directory
            }
        });
    assert!(
        lib_dir.join(filename).exists(),
        "Missing {} for {target}. Run scripts/bootstrap.ps1 (Windows) or bash scripts/bootstrap.sh (macOS/Linux) first.",
        lib_dir.join(filename).display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        lib_dir.join(filename).display()
    );
    cc::Build::new()
        .file("src/bridge.c")
        .include(source.join("include"))
        .define("GHOSTTY_STATIC", None)
        .compile("ghostlink-bridge");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static={library}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=m");
    }
}
