fn main() {
    println!("cargo:rerun-if-env-changed=GHOSTTY_DIR");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rerun-if-changed=src/bridge.h");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::env::var_os("GHOSTTY_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".deps/ghostty-f64f4aca2c29b554d111b36c3d946a9bddd159ff"));
    assert!(
        source.join("include/ghostty/vt.h").exists(),
        "Run ./scripts/bootstrap.ps1 first (missing Ghostty headers)."
    );
    assert!(
        source.join("zig-out/lib/ghostty-vt-static.lib").exists(),
        "Run ./scripts/bootstrap.ps1 first (missing native terminal library)."
    );
    cc::Build::new()
        .file("src/bridge.c")
        .include(source.join("include"))
        .define("GHOSTTY_STATIC", None)
        .compile("ghostlink-bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        source.join("zig-out/lib").display()
    );
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
}
