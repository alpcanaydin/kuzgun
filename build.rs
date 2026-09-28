// Embed an Info.plist in the bare binary so a `cargo run` build is named
// "Tusk" in the menu bar (Tusk.app has its own Info.plist). Name keys only:
// a bundle identifier would make the notification center expect a real bundle.
fn main() {
    let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/dev-Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}
