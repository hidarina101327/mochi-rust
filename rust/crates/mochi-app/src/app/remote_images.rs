//! 文档网络图片只为当前可见区域加载，最多 4 个并发请求；离开视口即释放字节缓存。
use super::*;
#[derive(Default)]
pub(super) struct State {
    visible: HashSet<String>,
    pending: HashSet<String>,
    failed: HashSet<String>,
}
impl App {
    pub(super) fn sync_remote_images(&mut self) {
        let visible: HashSet<String> = self
            .list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                crate::ui::draw::DrawCmd::Image { src, .. }
                    if src.starts_with("https://") || src.starts_with("http://") =>
                {
                    Some(src.clone())
                }
                _ => None,
            })
            .collect();
        for old in self.remote_images.visible.difference(&visible) {
            self.renderer.forget_bytes(old);
        }
        self.remote_images
            .failed
            .retain(|url| visible.contains(url));
        self.remote_images.visible = visible;
        for url in self.remote_images.visible.iter() {
            if self.remote_images.pending.len() >= 4 {
                break;
            }
            if self.remote_images.failed.contains(url)
                || self.remote_images.pending.contains(url)
                || self.renderer.has_bytes(url)
            {
                continue;
            }
            self.remote_images.pending.insert(url.clone());
            let url = url.clone();
            self.file_jobs
                .submit(PathBuf::new(), self.hwnd_raw, move || {
                    let result = mochi_core::link_files::fetch_image(&url)
                        .and_then(|bytes| {
                            let (w, h) = crate::ui::imginfo::dimensions_from_bytes(&bytes)
                                .ok_or_else(|| anyhow::anyhow!("无法识别图片格式"))?;
                            if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 32_000_000 {
                                anyhow::bail!("图片尺寸超过限制")
                            }
                            Ok((bytes, (w, h)))
                        })
                        .map_err(|e| e.to_string());
                    Ok(crate::file_runtime::Payload::Image { url, result })
                });
        }
    }
    pub(super) fn finish_remote_image(
        &mut self,
        url: String,
        result: std::result::Result<(Vec<u8>, (u32, u32)), String>,
    ) {
        self.remote_images.pending.remove(&url);
        if !self.remote_images.visible.contains(&url) {
            return;
        }
        match result {
            Ok((bytes, size)) => {
                self.renderer.register_bytes(&url, bytes);
                self.doc.set_image_size(&url, size);
                self.split.doc.set_image_size(&url, size);
            }
            Err(_) => {
                self.remote_images.failed.insert(url);
            }
        }
    }
}
