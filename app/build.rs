fn main() {
    // Debug builds keep element ids, which the headless UI tests look elements up by.
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()).with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");

    // The Windows executable carries the app icon and version information.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../assets/icons/icon.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../assets/icons/icon.ico").set("ProductName", "ViralFlow").set("FileDescription", "ViralFlow - video editor");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=the Windows icon could not be embedded: {error}");
        }
    }
}
