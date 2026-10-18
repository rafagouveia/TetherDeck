fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/tetherdeck.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/brand/tetherdeck.ico");
        resource.set("ProductName", "TetherDeck");
        resource.set("FileDescription", "TetherDeck · GPUI device workspace");
        if let Err(error) = resource.compile() {
            eprintln!("cargo:warning=Windows resource: {error}");
        }
    }
}
