fn main() {
    println!("cargo:rerun-if-changed=assets/apollo.rc");
    println!("cargo:rerun-if-changed=assets/apollo.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/apollo.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("Apollo's Windows icon resource must compile");
    }
}
