fn main() {
    // The UI only exists on Windows; elsewhere the binary just explains that,
    // so the workspace (and its portable crates' tests) builds on any OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        slint_build::compile("ui/app.slint").expect("Slint UI failed to compile");
    }
    println!("cargo:rerun-if-changed=ui");
}
