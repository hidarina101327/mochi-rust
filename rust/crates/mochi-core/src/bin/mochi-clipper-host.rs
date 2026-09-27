//! 实现浏览器剪藏扩展与 Mochi 之间的原生消息通信入口。
fn main() {
    #[cfg(windows)]
    if let Err(error) = mochi_core::web_clipper::native::host_main() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
