fn main() {
    // The UI only exists on Windows; elsewhere the binary just explains that,
    // so the workspace (and its portable crates' tests) builds on any OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Spanish texts in the .slint files are the source; English, French
        // and Russian come from lang/<locale>/LC_MESSAGES/familiar-core.po.
        let config = slint_build::CompilerConfiguration::new()
            .with_bundled_translations("lang")
            .with_default_translation_context(slint_build::DefaultTranslationContext::None);
        slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");
    }
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=lang");
}
