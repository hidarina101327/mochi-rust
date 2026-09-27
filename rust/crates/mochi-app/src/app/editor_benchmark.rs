//! 通过真实的 App 控制器和 DirectWrite 得到可复现的输入耗时。
//! 只编辑快照的可丢弃工作区；给定的文件是只读的。
use super::*;

/// 一个小型的分阶段探针，让基准的内存数字可以互相比较，
/// 同时不把采样工作混进单键延迟测量里。
struct MemoryProbe {
    sampler: resource_usage::Sampler,
    stages: serde_json::Map<String, serde_json::Value>,
    peak_working_set_bytes: Option<u64>,
    peak_private_bytes: Option<u64>,
}

impl MemoryProbe {
    fn new() -> Self {
        Self {
            sampler: resource_usage::Sampler::new(),
            stages: serde_json::Map::new(),
            peak_working_set_bytes: None,
            peak_private_bytes: None,
        }
    }

    fn sample(&mut self, stage: &str) {
        let sample = self.sampler.sample();
        self.peak_working_set_bytes =
            max_optional(self.peak_working_set_bytes, sample.working_set_bytes);
        self.peak_private_bytes = max_optional(self.peak_private_bytes, sample.private_bytes);
        self.stages.insert(
            stage.to_owned(),
            serde_json::json!({
                "cpuPercent": sample.cpu_percent,
                "workingSetBytes": sample.working_set_bytes,
                "privateBytes": sample.private_bytes,
                "threadCount": sample.thread_count,
            }),
        );
    }

    fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "stages": self.stages,
            // 这些是各命名阶段观察到的峰值。采样刻意放在按键计时
            // 循环之外，各次运行之间的数值才有可比性，也不干扰输入延迟。
            "peakObserved": {
                "workingSetBytes": self.peak_working_set_bytes,
                "privateBytes": self.peak_private_bytes,
            },
        })
    }
}

fn max_optional(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn summarize(values: &[f64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::json!({ "samplesMs": [], "medianMs": null, "maxMs": null, "p95Ms": null });
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    serde_json::json!({
        "samplesMs": values,
        "medianMs": sorted[sorted.len() / 2],
        "maxMs": sorted.last(),
        "p95Ms": sorted[((sorted.len() as f64 * 0.95).ceil() as usize - 1).min(sorted.len() - 1)],
    })
}

fn generated_edit_char(step: usize) -> char {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    ALPHABET[step % ALPHABET.len()] as char
}

impl App {
    // 导航可能替换挂载的内容页。断言几何或测量跳转之前，等的是这个页，
    // 不是没访问过的页。
    fn benchmark_finish_document_page(&mut self) -> Result<()> {
        self.paint(HWND::default())?;
        for _ in 0..10_000 {
            if !self.doc.is_loading() {
                return Ok(());
            }
            self.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
            self.paint(HWND::default())?;
        }
        Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "Content page exceeded the benchmark layout-step limit",
        ))
    }

    pub(super) fn benchmark_editor(
        &mut self,
        input: &Path,
        folder: &Path,
        output: &Path,
        snapshot: &gfx::Snapshot,
    ) -> Result<()> {
        let error = |e: anyhow::Error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        };
        let original = std::fs::read_to_string(input).map_err(|e| error(e.into()))?;
        let path = folder.join("editor-benchmark.md");
        std::fs::write(&path, &original).map_err(|e| error(e.into()))?;
        // 基准夹具要忠实于给定的文档：编辑器在绘制阶段解析行内评论锚点，
        // 只拷贝 Markdown 会悄悄跳过编辑真实文档所需的工作。通过评论数据文件的约定
        // 读写，才能保留锚点、回复及其关联元数据；源文档的评论数据文件
        // 绝不作为写入目标。
        let input_name = input.to_string_lossy();
        let source_comment_path = PathBuf::from(sidecars::comment_sidecar_path(&input_name));
        let source_comment_bytes = std::fs::read(&source_comment_path).ok();
        let source_comments = sidecars::load_comments(&input_name).comments;
        if source_comment_bytes.is_some() {
            sidecars::save_comments(&path.to_string_lossy(), source_comments.clone())
                .map_err(error)?;
        }
        let mut memory = MemoryProbe::new();
        memory.sample("beforeOpen");
        // 快照运行用的是 HWND::default()，常规的窗口过程钩子帮不了我们
        // 开启渐进布局。这个基准必须与真实大文档窗口走在同一条
        // 有界布局路径上。
        self.doc.set_progressive(true);
        let start = std::time::Instant::now();
        if !self.shell.open_file(&path) {
            return Err(error(anyhow::anyhow!("Cannot open benchmark fixture")));
        }
        self.sync_state();
        self.paint(HWND::default())?;
        let open_ms = start.elapsed().as_secs_f64() * 1000.0;
        memory.sample("afterFirstPaint");
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| error(e.into()))?;
        }
        self.renderer
            .save_snapshot(snapshot, &output.with_extension("first.png"))?;
        let loading_on_first_frame = self.doc.is_loading();
        let content_chunk_count = self.doc.chunk_plan().map_or(1, |p| p.chunks.len());
        let content_chunked = self.doc.is_chunked();
        // 记录这个夹具是否真的产生了让位。仅凭字节大小判断不出是否
        // 需要再切片：一个巨块是原子的，而多段落夹具有上千个让位机会。

        // 驱动窗口过程本会分发的同一个定时器回调，且不 sleep：
        // 每个样本只包含同步的 on_timer + 绘制工作。
        // 定时器的 OS 等待刻意排除在测量之外。
        const MAX_LOADING_STEPS: usize = 20_000;
        let mut loading_steps = Vec::new();
        let remaining_start = std::time::Instant::now();
        while self.doc.is_loading() {
            if loading_steps.len() >= MAX_LOADING_STEPS {
                return Err(error(anyhow::anyhow!(format!(
                    "progressive layout exceeded {MAX_LOADING_STEPS} timer steps"
                ))));
            }
            let step_start = std::time::Instant::now();
            self.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
            self.paint(HWND::default())?;
            loading_steps.push(step_start.elapsed().as_secs_f64() * 1000.0);
        }
        let remaining_layout_ms = remaining_start.elapsed().as_secs_f64() * 1000.0;
        // 布局计时不包含首帧 PNG 编码和采样。
        let complete_layout_ms = open_ms + remaining_layout_ms;
        memory.sample("afterCompleteLayout");
        self.focus = Focus::Main;
        self.editor_engaged = true;
        // 在中段一个普通段落里打字，避开 Markdown 语法。
        let at = original
            .match_indices("The quick")
            .nth(original.matches("The quick").count() / 2)
            .map(|(offset, _)| offset)
            .unwrap_or(0);
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(at, false);
        self.after_doc_selection_change(true);
        self.benchmark_finish_document_page()?;

        // 沿用渲染后的评论流程，而不只是读取和写入评论数据文件。解析器刻意允许
        // 旧锚点保持未解析状态——这些按未解析上报，而不是让一份
        // 合法的旧文档挂掉基准。但凡是能解析的锚点，都必须在真实
        // 光标滚动路径和绘制之后产生一个可见的页边气泡。
        let comment_checks: Vec<_> = self
            .panels
            .comments
            .comments
            .iter()
            .filter(|comment| comment.parent_id.is_none())
            .filter_map(|comment| {
                comment
                    .anchor
                    .as_ref()
                    .map(|anchor| (comment.id.clone(), anchor.clone()))
            })
            .collect();
        let comment_check_cursor = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .map(|buffer| buffer.cursor())
            .unwrap_or(0);
        let comment_check_scroll = self.shell.active_scroll();
        let mut resolved_comment_count = 0usize;
        let mut comment_marks_verified = 0usize;
        for (id, anchor) in &comment_checks {
            let Some(range) = self.comment_ranges.range_for(id).cloned() else {
                continue;
            };
            resolved_comment_count += 1;
            if let Some(buffer) = self.shell.active_buffer_mut() {
                buffer.set_cursor(range.start, false);
            } else {
                return Err(error(anyhow::anyhow!(
                    "Benchmark buffer disappeared during comment check"
                )));
            }
            self.after_doc_selection_change(false);
            self.benchmark_finish_document_page()?;
            if !self
                .comment_bubbles
                .iter()
                .any(|(_, bubble_id)| bubble_id == id)
            {
                return Err(error(anyhow::anyhow!(format!(
                    "Resolved comment {id} did not produce a visible bubble"
                ))));
            }
            if comment_marks_verified == 0 {
                self.renderer
                    .save_snapshot(snapshot, &output.with_extension("comment.png"))?;
            }
            // 旧锚点可能按前缀/偏移解析成功，哪怕当初选中的文字已经不存在。
            // 只有原始源码仍含有该文字时才校验选中文字；这样对完好的
            // 夹具检查依旧严格，又不至于拒绝这些合理可恢复的旧锚点。
            if !anchor.selected_text.is_empty() && original.contains(&anchor.selected_text) {
                let resolved_text = original.get(range.clone()).unwrap_or("");
                if !resolved_text.contains(&anchor.selected_text) {
                    return Err(error(anyhow::anyhow!(format!(
                        "Comment {id} resolved to the wrong selected text"
                    ))));
                }
            }
            comment_marks_verified += 1;
        }
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(comment_check_cursor, false);
        }
        self.after_doc_selection_change(true);
        self.benchmark_finish_document_page()?;
        self.shell.set_active_scroll(comment_check_scroll);
        self.paint(HWND::default())?;

        // 与窗口过程走同一条滚轮路径，并像 Windows 那样成批处理消息。
        // 测量一段长距离滚动加它产生的帧，而不是几千个冗余的中间帧。
        let area = self.editor_area;
        if area.is_empty() {
            return Err(error(anyhow::anyhow!("Editor area is empty")));
        }
        let wheel_x = (area.left + area.right) * 0.5;
        let wheel_y = (area.top + area.bottom) * 0.5;
        let max_scroll = self.doc.max_scroll(area);
        let down_step = theme::ROW_HEIGHT * 3.0 * (f32::from(i16::MIN).abs() / 120.0);
        let up_step = theme::ROW_HEIGHT * 3.0 * (f32::from(i16::MAX) / 120.0);
        let wheel_count = |distance: f32, step: f32| (distance / step).ceil().max(1.0) as usize;
        let mut scrolling = Vec::with_capacity(2);
        let started = std::time::Instant::now();
        for _ in 0..wheel_count(max_scroll, down_step) {
            self.on_wheel(wheel_x, wheel_y, i16::MIN);
        }
        self.paint(HWND::default())?;
        scrolling.push(started.elapsed().as_secs_f64() * 1000.0);
        let middle_scroll = max_scroll * 0.5;
        let started = std::time::Instant::now();
        for _ in 0..wheel_count(
            (self.shell.active_scroll() - middle_scroll).max(0.0),
            up_step,
        ) {
            self.on_wheel(wheel_x, wheel_y, i16::MAX);
        }
        self.paint(HWND::default())?;
        scrolling.push(started.elapsed().as_secs_f64() * 1000.0);

        // Ctrl+End 是真实的文档跳转路径：移动缓冲光标、只重建受影响的
        // 编辑器状态、把光标滚进视野。回到中段则用普通编辑输入同样的
        // 选区/滚动后续，不逐行走一遍。
        let mut jumping = Vec::with_capacity(2);
        let started = std::time::Instant::now();
        if !self.on_edit_key(
            windows::Win32::UI::Input::KeyboardAndMouse::VK_END.0,
            false,
            true,
        ) {
            return Err(error(anyhow::anyhow!(
                "App rejected Ctrl+End benchmark jump"
            )));
        }
        self.benchmark_finish_document_page()?;
        jumping.push(started.elapsed().as_secs_f64() * 1000.0);
        let tail_preserved = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .is_some_and(|buffer| buffer.cursor() == original.len() && buffer.text() == original);
        if !tail_preserved {
            return Err(error(anyhow::anyhow!(
                "Ctrl+End did not retain the complete benchmark tail"
            )));
        }

        let started = std::time::Instant::now();
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(at, false);
        } else {
            return Err(error(anyhow::anyhow!(
                "Benchmark buffer disappeared after Ctrl+End"
            )));
        }
        self.after_doc_selection_change(true);
        self.benchmark_finish_document_page()?;
        jumping.push(started.elapsed().as_secs_f64() * 1000.0);

        let mut typing = Vec::new();
        let mut typing_input = Vec::new();
        let mut typing_paint = Vec::new();
        for ch in "native latency".chars() {
            let start = std::time::Instant::now();
            if !self.on_char(ch) {
                return Err(error(anyhow::anyhow!("App rejected typed character")));
            }
            typing_input.push(start.elapsed().as_secs_f64() * 1000.0);
            let paint_start = std::time::Instant::now();
            self.paint(HWND::default())?;
            typing_paint.push(paint_start.elapsed().as_secs_f64() * 1000.0);
            typing.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        memory.sample("afterContinuousInput");
        let mut arrows = Vec::new();
        for _ in 0..12 {
            let start = std::time::Instant::now();
            self.on_edit_key(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT.0,
                false,
                false,
            );
            self.paint(HWND::default())?;
            arrows.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        // 撤销里可能有富文本插入产生的多个组。每组都必须精确还原。
        for _ in 0..32 {
            if self.shell.active().unwrap().buffer().unwrap().text() == original {
                break;
            }
            self.undo_redo(false);
        }
        let restored = self.shell.active().unwrap().buffer().unwrap().text() == original;
        if !restored {
            return Err(error(anyhow::anyhow!(
                "Typing undo did not restore the source"
            )));
        }
        memory.sample("afterUndo");
        self.on_ime_composition("zhongwen", 8);
        self.on_ime_cancel();
        if self.shell.active().unwrap().buffer().unwrap().text() != original {
            return Err(error(anyhow::anyhow!(
                "IME cancellation changed the source"
            )));
        }
        self.on_word_count_timer();
        self.paint(HWND::default())?;
        let fixture_comments = sidecars::load_comments(&path.to_string_lossy()).comments;
        let source_comment_sidecar_unchanged = match source_comment_bytes.as_deref() {
            Some(before) => std::fs::read(&source_comment_path)
                .map(|after| after == before)
                .unwrap_or(false),
            None => !source_comment_path.exists(),
        };
        let comment_integrity = source_comments == fixture_comments;
        let comment_count = source_comments.len();
        let anchored_count = source_comments
            .iter()
            .filter(|comment| comment.anchor.is_some())
            .count();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| error(e.into()))?;
        }
        let report = serde_json::json!({
            "bytes": original.len(), "openAndFirstPaintMs": open_ms,
            "loadingOnFirstFrame": loading_on_first_frame,
            "loadingStepAndPaint": summarize(&loading_steps),
            "loadingStepCount": loading_steps.len(),
            "contentChunked": content_chunked,
            "contentChunkCount": content_chunk_count,
            "layoutScope": if content_chunked { "initial content page only" } else { "whole document" },
            "completeLayoutMs": complete_layout_ms,
            "remainingLayoutMs": remaining_layout_ms,
            "typingAndPaint": summarize(&typing), "arrowAndPaint": summarize(&arrows),
            "typingInput": summarize(&typing_input), "typingPaint": summarize(&typing_paint),
            "scrollAndPaint": summarize(&scrolling), "jumpAndPaint": summarize(&jumping),
            "tailPreservedAfterJump": tail_preserved,
            "undoRestoredSource": restored, "imeCancelPreservedSource": true,
            "inputFileUnchanged": std::fs::read_to_string(input).map_err(|e| error(e.into()))? == original,
            "commentCount": comment_count,
            "anchoredCount": anchored_count,
            "commentIntegrity": comment_integrity,
            "resolvedCommentCount": resolved_comment_count,
            "commentMarksVerified": comment_marks_verified,
            "sourceCommentSidecarPresent": source_comment_bytes.is_some(),
            "sourceCommentSidecarUnchanged": source_comment_sidecar_unchanged,
            "resourceMemory": memory.report(),
            "debugAssertions": cfg!(debug_assertions),
            "mode": "isolated App controller, real DirectWrite/software Direct2D, no OS input latency; loading samples are synchronous on_timer+paint work and exclude OS timer wait"
        });
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .map_err(|e| error(e.into()))?;
        println!("{report}");
        Ok(())
    }

    /// 长时间运行的编辑器探针，用来对比不同原生修订版的内存与输入表现。
    /// 源夹具复制到快照的可丢弃工作区；给定的输入文件绝不编辑。
    pub(super) fn benchmark_editor_memory(
        &mut self,
        input: &Path,
        folder: &Path,
        output: &Path,
    ) -> Result<()> {
        let error = |e: anyhow::Error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        };
        let original = std::fs::read_to_string(input).map_err(|e| error(e.into()))?;
        let path = folder.join("editor-memory-benchmark.md");
        std::fs::write(&path, &original).map_err(|e| error(e.into()))?;

        let mut memory = MemoryProbe::new();
        memory.sample("beforeOpen");
        let open_start = std::time::Instant::now();
        if !self.shell.open_file(&path) {
            return Err(error(anyhow::anyhow!(
                "Cannot open memory benchmark fixture"
            )));
        }
        self.sync_state();
        self.focus = Focus::Main;
        self.editor_engaged = true;
        let at = original
            .match_indices("The quick")
            .nth(original.matches("The quick").count() / 2)
            .map(|(offset, _)| offset)
            .unwrap_or(0);
        self.shell
            .active_buffer_mut()
            .ok_or_else(|| error(anyhow::anyhow!("Memory benchmark buffer missing")))?
            .set_cursor(at, false);
        self.paint(HWND::default())?;
        let open_ms = open_start.elapsed().as_secs_f64() * 1000.0;
        memory.sample("afterFirstPaint");

        // 每轮五处分散的字符编辑产生 200 条独立历史记录，
        // 方向键则锻炼真实的光标移动。
        const CYCLES: usize = 40;
        const EDITS_PER_CYCLE: usize = 5;
        const EDIT_COUNT: usize = CYCLES * EDITS_PER_CYCLE;
        let mut typing = Vec::with_capacity(EDIT_COUNT);
        let mut movement = Vec::with_capacity(EDIT_COUNT);
        for step in 0..EDIT_COUNT {
            let started = std::time::Instant::now();
            if !self.on_char(generated_edit_char(step)) {
                return Err(error(anyhow::anyhow!("App rejected generated edit")));
            }
            self.paint(HWND::default())?;
            typing.push(started.elapsed().as_secs_f64() * 1000.0);

            let started = std::time::Instant::now();
            if !self.on_edit_key(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT.0,
                false,
                false,
            ) {
                return Err(error(anyhow::anyhow!(
                    "App rejected generated cursor movement"
                )));
            }
            self.paint(HWND::default())?;
            movement.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        memory.sample("afterContinuousInput");

        let mut undo = Vec::with_capacity(EDIT_COUNT);
        let mut undo_steps = 0usize;
        for _ in 0..EDIT_COUNT {
            let before_len = self
                .shell
                .active()
                .and_then(|tab| tab.buffer())
                .map(|buffer| buffer.text().len())
                .ok_or_else(|| error(anyhow::anyhow!("Memory benchmark buffer missing")))?;
            let started = std::time::Instant::now();
            if !self.on_shortcut(HWND::default(), b'Z' as u16, false, true) {
                return Err(error(anyhow::anyhow!("App rejected generated undo")));
            }
            self.paint(HWND::default())?;
            undo.push(started.elapsed().as_secs_f64() * 1000.0);
            let after = self
                .shell
                .active()
                .and_then(|tab| tab.buffer())
                .map(|buffer| buffer.text().len())
                .ok_or_else(|| error(anyhow::anyhow!("Memory benchmark buffer missing")))?;
            if after.saturating_add(1) != before_len {
                break;
            }
            undo_steps += 1;
        }
        let restored = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .is_some_and(|buffer| buffer.text() == original);
        if !restored || undo_steps != EDIT_COUNT {
            return Err(error(anyhow::anyhow!(format!(
                "200-step editor history failed: restored={restored}, undoSteps={undo_steps}"
            ))));
        }

        // 不允许出现第 201 条可撤销的编辑。在保持 200 条上限的同时，
        // 守住历史契约。
        if !self.on_shortcut(HWND::default(), b'Z' as u16, false, true) {
            return Err(error(anyhow::anyhow!("App rejected final undo probe")));
        }
        let no_extra_undo = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .is_some_and(|buffer| buffer.text() == original);
        if !no_extra_undo {
            return Err(error(anyhow::anyhow!(
                "Undo history changed after 200 steps"
            )));
        }
        memory.sample("afterUndo");

        // 重新打开这份可丢弃的基准副本，暴露关闭的编辑器页签
        // 遗留的资源。close_tab 只保存这份生成的副本。
        let active = self
            .shell
            .active_tab()
            .ok_or_else(|| error(anyhow::anyhow!("Memory benchmark tab missing")))?;
        self.shell.close_tab(active);
        self.doc.invalidate();
        self.sync_state();
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.paint(HWND::default())?;
        memory.sample("afterClose");
        let reopen_start = std::time::Instant::now();
        if !self.shell.open_file(&path) {
            return Err(error(anyhow::anyhow!(
                "Cannot reopen memory benchmark fixture"
            )));
        }
        self.sync_state();
        self.paint(HWND::default())?;
        let reopen_ms = reopen_start.elapsed().as_secs_f64() * 1000.0;
        let reopened_original = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .is_some_and(|buffer| buffer.text() == original);
        if !reopened_original {
            return Err(error(anyhow::anyhow!("Reopened benchmark source changed")));
        }
        memory.sample("afterReopen");

        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| error(e.into()))?;
        }
        let report = serde_json::json!({
            "bytes": original.len(),
            "openAndFirstPaintMs": open_ms,
            "reopenAndFirstPaintMs": reopen_ms,
            "generated": {
                "cycles": CYCLES,
                "editsPerCycle": EDITS_PER_CYCLE,
                "editCount": EDIT_COUNT,
                "undoSteps": undo_steps,
                "undoLimit": 200,
                "restoredSource": restored,
                "noExtraUndo": no_extra_undo,
                "reopenedOriginal": reopened_original,
            },
            "typingAndPaint": summarize(&typing),
            "movementAndPaint": summarize(&movement),
            "undoAndPaint": summarize(&undo),
            "resourceMemory": memory.report(),
            "inputFileUnchanged": std::fs::read_to_string(input)
                .map_err(|e| error(e.into()))?
                == original,
            "mode": "Debug, isolated App controller, real DirectWrite/software Direct2D, generated 40x5 edit/movement/undo groups",
        });
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .map_err(|e| error(e.into()))?;
        println!("{report}");
        Ok(())
    }
}
