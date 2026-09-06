fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/app-icon/hodoq.rc");
    println!("cargo:rerun-if-changed=assets/app-icon/hodoq.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // GPUI loads the window/taskbar icon from resource ID 1 in the executable.
        embed_resource::compile_for(
            "assets/app-icon/hodoq.rc",
            ["hodoq"],
            embed_resource::ParamsIncludeDirs(["assets/app-icon"]),
        )
        .manifest_required()
        .expect("failed to embed the HodoQ application icon");
    }
}
