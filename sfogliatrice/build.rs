fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", "CLI tool to tessellate GeoJSON geometries into satellite survey targets and coverages");
        res.set("ProductName", "sfogliatrice");
        res.set("LegalCopyright", "Copyright (c) 2026 Ronaldo Ferreira");
        res.set("CompanyName", "Ronaldo Ferreira");
        res.set("InternalName", "sfogliatrice.exe");
        res.set("OriginalFilename", "sfogliatrice.exe");
        res.set("Comments", "https://github.com/racum/sfogliatrice");
        res.set_language(0x0409); // English (United States)
        res.set_icon("../assets/sfogliatrice.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}
