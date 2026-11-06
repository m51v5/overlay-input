fn main() {
    println!("cargo:rerun-if-changed=assets/windows/app.rc");
    println!("cargo:rerun-if-changed=assets/windows/app.manifest");
    println!("cargo:rerun-if-changed=assets/windows/app.ico");

    embed_resource::compile("assets/windows/app.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
