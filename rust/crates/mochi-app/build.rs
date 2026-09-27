//! 在 Windows 构建时嵌入应用图标，并配置许可证文件的打包方式。
fn main() {
    // 生成 build/icon.ico。
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("../../../build/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if icon.is_file() {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let resource = out.join("mochi-app.rc");
        let icon_path = icon.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(&resource, format!("101 ICON \"{icon_path}\"\n"))
            .expect("write Windows icon resource");
        embed_resource::compile(&resource, embed_resource::NONE)
            .manifest_optional()
            .expect("embed Windows icon resource");
    } else {
        println!("cargo:warning=build/icon.ico is missing; run `node scripts/generate-icons.mjs` before packaging");
    }

    // 打包时将 licenses/ 目录与 mochi-app.exe 一并包含。
    println!("cargo:rerun-if-changed=assets/licenses");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    if let Some(profile) = out.ancestors().nth(3) {
        let destination = profile.join("licenses");
        std::fs::create_dir_all(&destination).expect("create license output");
        for name in [
            "ratex-MIT.txt",
            "katex-MIT.txt",
            "katex-OFL.txt",
            "katex-FONT-NOTICE.txt",
            "lucide-ISC.txt",
        ] {
            std::fs::copy(
                std::path::Path::new("assets/licenses").join(name),
                destination.join(name),
            )
            .expect("copy math license");
        }
    }
    // 本包只链接可执行程序/测试，不发布 DLL；无需 MSVC 的导入库和导出文件。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/NOIMPLIB");
        println!("cargo:rustc-link-arg=/NOEXP");
    }
}
