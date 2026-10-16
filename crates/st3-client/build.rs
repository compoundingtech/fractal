fn main() {
    println!("cargo:rerun-if-changed=native/client_mode_acl.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/client_mode_acl.c")
            .flag("-std=c11")
            .flag("-Werror=switch-enum")
            .warnings(true)
            .compile("st_client_mode_acl");
    }
}
