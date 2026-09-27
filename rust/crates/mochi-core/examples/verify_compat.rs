//! 逐字节核对 Electron JSON 往返。不要调用工作区初始化接口，避免写入或迁移真实数据。

use std::path::{Path, PathBuf};

use mochi_core::agenda::AgendaData;
use mochi_core::ai::{AiConversation, AiDocumentMountIndex, AiSessionIndex};
use mochi_core::analytics::ActivityEvent;
use mochi_core::capture::CaptureFile;
use mochi_core::domain::{AiPermissionsConfig, Library, LibraryType, MochiConfig, SidebarConfig};
use mochi_core::{json2, paths};

/// 两个写入方的落盘约定不同，必须分别校验。
#[derive(Clone, Copy, PartialEq)]
enum Trailer {
    /// `.mochi/*.json`：`writeFile(JSON.stringify(v, null, 2))`，无尾换行
    None,
    /// `agenda/agenda.json`、`收件箱/*.json`：`writeFile(JSON.stringify(v, null, 2) + '\n')`
    Newline,
}

struct Report {
    checked: u32,
    failed: u32,
}

fn main() {
    let Some(root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("用法: verify_compat <工作区路径>");
        std::process::exit(2);
    };
    if !root.is_dir() {
        eprintln!("工作区不存在: {}", root.display());
        std::process::exit(2);
    }

    println!("工作区: {}\n", root.display());
    let mut r = Report {
        checked: 0,
        failed: 0,
    };

    println!(".mochi/  （无尾换行）");
    check::<MochiConfig>(&paths::config_file(&root), Trailer::None, &mut r);
    check::<SidebarConfig>(&paths::sidebar_file(&root), Trailer::None, &mut r);
    check::<AiPermissionsConfig>(&paths::ai_permissions_file(&root), Trailer::None, &mut r);
    check::<Vec<LibraryType>>(&paths::library_types_file(&root), Trailer::None, &mut r);
    check::<Vec<Library>>(&paths::libraries_file(&root), Trailer::None, &mut r);

    println!("\nagenda/  （带尾换行）");
    check::<AgendaData>(
        &mochi_core::agenda::AgendaStore::new(&root).data_path(),
        Trailer::Newline,
        &mut r,
    );

    println!("\n收件箱/  （带尾换行）");
    check::<CaptureFile>(
        &root.join(paths::INBOX_DIR_NAME).join("items.json"),
        Trailer::Newline,
        &mut r,
    );

    let sessions_root = paths::mochi_dir(&root).join("ai-sessions");
    println!("\n.mochi/ai-sessions/  （无尾换行）");
    check::<AiSessionIndex>(&sessions_root.join("index.json"), Trailer::None, &mut r);
    check_document_mounts(&sessions_root.join("document-mounts.json"), &mut r);
    check_all_sessions(&sessions_root.join("sessions"), &mut r);

    println!("\n.mochi/activity/  （JSONL，逐行）");
    check_activity_log(&paths::mochi_dir(&root).join("activity"), &mut r);

    println!();
    if r.checked == 0 {
        eprintln!("✗ 没有找到可校验的数据文件，不能判定兼容");
        std::process::exit(1);
    } else if r.failed == 0 {
        println!("✓ {} 个文件全部字节级一致", r.checked);
    } else {
        println!("✗ {}/{} 个文件不一致", r.failed, r.checked);
        std::process::exit(1);
    }
}

fn check<T>(path: &Path, trailer: Trailer, r: &mut Report)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let Some(original) = read_original(path, r) else {
        return;
    };

    let parsed: T = match json2::deserialize(&original) {
        Ok(v) => v,
        Err(e) => {
            r.failed += 1;
            println!("  ✗ {name:<22} 解析失败: {e}");
            return;
        }
    };
    let body = match json2::serialize(&parsed) {
        Ok(s) => s,
        Err(e) => {
            r.failed += 1;
            println!("  ✗ {name:<22} 序列化失败: {e}");
            return;
        }
    };
    let rewritten = match trailer {
        Trailer::None => body,
        Trailer::Newline => format!("{body}\n"),
    };

    if original == rewritten {
        println!("  ✓ {name:<22} {} 字节", rewritten.len());
        return;
    }

    r.failed += 1;
    // 单独点名尾换行差异，否则只会看到一个很难读的"末尾不一致"
    if original.trim_end_matches('\n') == rewritten.trim_end_matches('\n') {
        println!(
            "  ✗ {name:<22} 仅尾换行不同（磁盘 {} 个，重写 {} 个）",
            original.len() - original.trim_end_matches('\n').len(),
            rewritten.len() - rewritten.trim_end_matches('\n').len()
        );
        return;
    }
    println!("  ✗ {name:<22} 不一致");
    print_first_diff(&original, &rewritten);
}

/// 挂载索引不能走 `check::<T>`：`AiDocumentMountIndex` 只派生 `Serialize`，
/// 反序列化走 `from_value`（逐条容错，坏一条不至于丢掉整个索引）。
///
/// 这个文件是保序模型的重点验证对象——真实数据里同一个文件存在三种键顺序，
/// 用固定字段的结构体读写一遍会把它们全部重排。
fn check_document_mounts(path: &Path, r: &mut Report) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let Some(original) = read_original(path, r) else {
        return;
    };

    let Ok(value) = serde_json::from_str::<serde_json::Value>(&original) else {
        r.failed += 1;
        println!("  ✗ {name:<22} 解析失败");
        return;
    };
    let index = AiDocumentMountIndex::from_value(&value);
    let rewritten = json2::serialize(&index).unwrap_or_default();

    if original == rewritten {
        let orders: std::collections::BTreeSet<String> = index
            .mounts
            .iter()
            .map(|m| m.fields().keys().cloned().collect::<Vec<_>>().join(","))
            .collect();
        println!(
            "  ✓ {name:<22} {} 字节（{} 条挂载，{} 种键顺序）",
            rewritten.len(),
            index.mounts.len(),
            orders.len()
        );
        return;
    }

    r.failed += 1;
    println!("  ✗ {name:<22} 不一致");
    print_first_diff(&original, &rewritten);
}

/// 活动日志是 JSONL，逐行校验。行数以万计，只汇总输出，失败的才点名。
///
/// 这里同时是对「固定结构体够不够用」的实证：真实数据里有 6 种键顺序，
/// 但它们都是同一个规范顺序的子序列，`skip_serializing_if` 就能全部复现。
/// 一旦有哪条记录带了新字段或换了顺序，这里会立刻报出来。
fn check_activity_log(dir: &Path, r: &mut Report) {
    let files = collect_files(dir, "jsonl", r);

    let mut total_lines = 0usize;
    let mut bad_lines = 0usize;
    let mut shapes = std::collections::BTreeSet::new();

    for file in &files {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        let Some(content) = read_original(file, r) else {
            continue;
        };

        let mut file_bad = 0usize;
        let mut lines = 0usize;
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            lines += 1;
            total_lines += 1;

            let parsed = serde_json::from_str::<ActivityEvent>(line);
            let rewritten = parsed
                .as_ref()
                .ok()
                .and_then(|e| serde_json::to_string(e).ok());
            match rewritten {
                Some(out) if out == line => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(object) = value.as_object() {
                            shapes.insert(object.keys().cloned().collect::<Vec<_>>().join(","));
                        }
                    }
                }
                Some(out) => {
                    file_bad += 1;
                    if file_bad == 1 {
                        println!("  ✗ {name:<22} 第 {} 行不一致", index + 1);
                        print_first_diff(line, &out);
                    }
                }
                None => {
                    file_bad += 1;
                    if file_bad == 1 {
                        println!("  ✗ {name:<22} 第 {} 行解析失败", index + 1);
                        println!("      {}", snippet(line, 0, line.len().min(160)));
                    }
                }
            }
        }

        bad_lines += file_bad;
        if file_bad == 0 {
            println!("  ✓ {name:<22} {lines} 行全部一致");
        } else {
            r.failed += 1;
            println!("  ✗ {name:<22} {file_bad}/{lines} 行不一致");
        }
    }

    if total_lines > 0 && bad_lines == 0 {
        println!(
            "    共 {total_lines} 行，{} 种键顺序，全部逐字节复现",
            shapes.len()
        );
    }
}

/// 会话文件可能有几十上百个，逐个校验但只汇总输出，失败的才点名。
fn check_all_sessions(dir: &Path, r: &mut Report) {
    let files = collect_files(dir, "json", r);

    if files.is_empty() {
        println!("  – sessions/               空目录，跳过");
        return;
    }

    let mut ok = 0u32;
    let mut bad: Vec<String> = Vec::new();
    for path in &files {
        r.checked += 1;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Ok(original) = std::fs::read_to_string(path) else {
            r.failed += 1;
            bad.push(format!("{name}（读不出来）"));
            continue;
        };
        match json2::deserialize::<AiConversation>(&original)
            .ok()
            .and_then(|c| json2::serialize(&c).ok())
        {
            Some(rewritten) if rewritten == original => ok += 1,
            _ => {
                r.failed += 1;
                bad.push(name);
            }
        }
    }

    println!(
        "  {} sessions/*.json         {ok}/{} 一致",
        if bad.is_empty() { "✓" } else { "✗" },
        files.len()
    );
    for name in bad.iter().take(5) {
        println!("      ✗ {name}");
    }
    if bad.len() > 5 {
        println!("      …另有 {} 个", bad.len() - 5);
    }
}

fn print_first_diff(a: &str, b: &str) {
    let at = a
        .char_indices()
        .zip(b.char_indices())
        .find(|((_, x), (_, y))| x != y)
        .map(|((i, _), _)| i)
        .unwrap_or(a.len().min(b.len()));
    let start = at.saturating_sub(60);
    println!("      首个差异 @ 字节 {at}");
    println!("      磁盘: …{}", snippet(a, start, at + 40));
    println!("      重写: …{}", snippet(b, start, at + 40));
}

/// 只有缺失的可选文件可以跳过。UTF-8 损坏、文件正被占用或
/// 权限错误都应导致失败，否则无法读取的数据集可能会被误判为“通过”。
fn collect_files(dir: &Path, extension: &str, r: &mut Report) -> Vec<PathBuf> {
    let mut files = Vec::new();
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let path = entry.path();
                        if path.extension().is_some_and(|e| e == extension) {
                            files.push(path);
                        }
                    }
                    Err(error) => {
                        r.failed += 1;
                        println!("  ✗ {} 目录项读取失败: {error}", dir.display());
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("  – {} 不存在，跳过", dir.display())
        }
        Err(error) => {
            r.failed += 1;
            println!("  ✗ {} 目录读取失败: {error}", dir.display());
        }
    }
    files.sort();
    files
}

fn read_original(path: &Path, r: &mut Report) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            r.checked += 1;
            Some(raw)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("  – {} 不存在，跳过", path.display());
            None
        }
        Err(error) => {
            r.checked += 1;
            r.failed += 1;
            println!("  ✗ {} 读取失败: {error}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_utf8_is_a_failure_not_a_missing_file() {
        let path =
            std::env::temp_dir().join(format!("mochi-compat-invalid-{}.json", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, &[0xff, 0xfe]).unwrap();
        drop(file);
        let mut report = Report {
            checked: 0,
            failed: 0,
        };
        check::<serde_json::Value>(&path, Trailer::None, &mut report);
        assert_eq!((report.checked, report.failed), (1, 1));
        std::fs::remove_file(&path).unwrap();
        assert!(read_original(&path, &mut report).is_none());
        assert_eq!((report.checked, report.failed), (1, 1));
    }
}

fn snippet(s: &str, start: usize, end: usize) -> String {
    let end = end.min(s.len());
    let start = start.min(end);
    s.get(start..end).unwrap_or("").replace('\n', "\\n")
}
