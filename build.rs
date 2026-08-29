use std::env;

fn main() {
    println!("cargo:rerun-if-changed=assets/apollo.rc");
    println!("cargo:rerun-if-changed=assets/apollo.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version =
            env::var("CARGO_PKG_VERSION").expect("Cargo package version must be available");
        let macros = [
            format!(
                "APOLLO_VERSION_MAJOR={}",
                env::var("CARGO_PKG_VERSION_MAJOR")
                    .expect("Cargo package major version must be available")
            ),
            format!(
                "APOLLO_VERSION_MINOR={}",
                env::var("CARGO_PKG_VERSION_MINOR")
                    .expect("Cargo package minor version must be available")
            ),
            format!(
                "APOLLO_VERSION_PATCH={}",
                env::var("CARGO_PKG_VERSION_PATCH")
                    .expect("Cargo package patch version must be available")
            ),
            format!("APOLLO_VERSION_STRING=\"{version}\""),
        ];

        embed_resource::compile("assets/apollo.rc", &macros)
            .manifest_optional()
            .expect("Apollo's Windows resources must compile");
    }
}
