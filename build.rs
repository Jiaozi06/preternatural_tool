fn main() -> std::io::Result<()> {
    const ICON: &str = "assets/toolbox.ico";
    const MANIFEST: &str = "assets/app.manifest";
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed={MANIFEST}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon(ICON)
            .set_manifest_file(MANIFEST)
            .set("ProductName", "绿玩变异工具箱 v1.2 ©️Jiaozi")
            .set("FileDescription", "绿玩变异工具箱 v1.2 ©️Jiaozi")
            .set("OriginalFilename", "绿玩变异工具箱 v1.2.exe")
            .set("LegalCopyright", "©️ Jiaozi");
        resource.compile()?;
    }
    Ok(())
}
