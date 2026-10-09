fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/app.rc");
    println!("cargo:rerun-if-changed=assets/till.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo package version is missing");
        let numeric_version = format!(
            "{},{},{},0",
            std::env::var("CARGO_PKG_VERSION_MAJOR").expect("missing major version"),
            std::env::var("CARGO_PKG_VERSION_MINOR").expect("missing minor version"),
            std::env::var("CARGO_PKG_VERSION_PATCH").expect("missing patch version")
        );
        let root = std::path::PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is missing"),
        );
        let icon = root
            .join("assets/till.ico")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string("packaging/windows/app.rc")
            .expect("unable to read Windows resource template")
            .replace("@ICON@", &icon)
            .replace("@VERSION@", &version)
            .replace("@NUMERIC_VERSION@", &numeric_version);
        let resource = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo output directory is missing"),
        )
        .join("app.rc");
        std::fs::write(&resource, source).expect("unable to write Windows resources");
        // Fail the build rather than silently shipping without the application identity.
        embed_resource::compile(&resource, embed_resource::NONE)
            .manifest_required()
            .expect("failed to compile Cash Ledger's Windows resources");
    }
}
