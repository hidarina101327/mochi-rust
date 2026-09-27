//! 配置 MSVC 链接参数，避免为核心库生成无用的导入库文件。
fn main() {
    // 仅作用于测试/示例等链接目标，rlib 本身不链接；不生成无用的 EXE 导入库。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/NOIMPLIB");
        println!("cargo:rustc-link-arg=/NOEXP");
    }
}
