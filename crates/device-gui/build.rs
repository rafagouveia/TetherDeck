fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/tetherdeck.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/brand/tetherdeck.ico")
            .set("ProductName", "TetherDeck")
            .set("FileDescription", "TetherDeck - iOS device workspace")
            .compile()
            .expect("compile TetherDeck Windows icon and version metadata");
    }
}
