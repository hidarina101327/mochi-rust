use super::*;
fn spec(language: Language, code: &str) -> ScriptSpec {
    ScriptSpec {
        language,
        code: code.into(),
        summary: "测试：临时目录内明确的样例".into(),
        intent: Intent::Inspect,
        input: Value::Null,
        cwd: None,
        timeout_ms: 5000,
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mochi-script-test-{}",
            crate::paths::random_base36(16)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn validation_rejects_invalid_or_unbounded_scripts() {
    let mut s = spec(Language::Python, "print('hello')");
    assert!(s.validate().is_ok());
    s.timeout_ms = 300001;
    assert!(s.validate().is_err());
    s.timeout_ms = 999;
    assert!(s.validate().is_err());
    s.timeout_ms = 5000;
    s.summary.clear();
    assert!(s.validate().is_err());
    s.summary = "测试".into();
    s.code = " ".into();
    assert!(s.validate().is_err());
    s.code = "\0".into();
    assert!(s.validate().is_err());
    s.code = "x".repeat(65537);
    assert!(s.validate().is_err());
    s.code = "print(1)".into();
    s.input = json!("x".repeat(1048577));
    assert!(s.validate().is_err());
    assert!(serde_json::from_value::<ScriptSpec>(
        json!({"language":"javascript","code":"x","summary":"x","intent":"inspect"})
    )
    .is_err());
    assert!(serde_json::from_value::<ScriptSpec>(
        json!({"language":"python","code":"x","summary":"x","intent":"preview","executable":"evil"})
    )
    .is_err());
}
#[test]
fn cwd_rejects_outside_or_non_directory() {
    let f = Fixture::new();
    let mut s = spec(Language::Python, "print(1)");
    assert_eq!(
        s.working_directory(&f.0).unwrap(),
        f.0.canonicalize().unwrap()
    );
    s.cwd = Some("..".into());
    assert!(s.working_directory(&f.0).is_err());
    fs::write(f.0.join("file"), "x").unwrap();
    s.cwd = Some("file".into());
    assert!(s.working_directory(&f.0).is_err());
    fs::create_dir(f.0.join("nested")).unwrap();
    s.cwd = Some("nested".into());
    assert!(s.working_directory(&f.0).is_ok());
}
#[test]
fn approval_source_and_cwd_must_match_exactly() {
    let mut s = spec(Language::Python, "print('原始审批内容')");
    s.cwd = Some("D:/workspace".into());
    let data = json!({"script":s});
    assert!(approved_spec(&data, &s.code, s.cwd.as_deref().unwrap()).is_ok());
    assert!(approved_spec(&data, "print('changed')", "D:/workspace").is_err());
    assert!(approved_spec(&data, &s.code, "D:/elsewhere").is_err());
    assert!(approved_spec(&json!({"kind":"script"}), &s.code, "D:/workspace").is_err());
}
#[test]
fn missing_bundle_never_falls_back_to_system_python() {
    let f = Fixture::new();
    assert!(runtime_path(Language::Python, &f.0)
        .unwrap_err()
        .contains("内置 Python"));
    let error = run(
        &spec(Language::Python, "print(1)"),
        &f.0,
        &f.0,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("内置 Python"));
}
#[test]
fn environment_is_honest_about_approval_and_isolation() {
    let env = environment();
    assert_eq!(env["sandboxed"], false);
    assert_eq!(env["requiresApproval"], true);
    assert_eq!(env["previewIsEnforcedReadOnly"], false);
    assert!(GUIDE.contains("不要用脚本绕过"));
    assert!(GUIDE.contains("哈希"));
}
#[cfg(windows)]
#[test]
fn windows_scripts_preserve_unicode_parameters_and_exit_codes() {
    let f = Fixture::new();
    let mut s=spec(Language::Powershell,"$data = Get-Content -LiteralPath $env:MOCHI_SCRIPT_INPUT -Raw -Encoding UTF8 | ConvertFrom-Json\nWrite-Output $data.name");
    s.input = json!({"name":"中文 & 引号 ' \" 不执行"});
    let result = run(&s, &f.0, &f.0, &AtomicBool::new(false)).unwrap();
    assert!(result.ok, "{result:?}");
    assert!(result.stdout.contains("中文 & 引号"));
    let result = run(
        &spec(
            Language::Cmd,
            "echo 中文输出\necho marker>cwd-marker.txt\nexit /b 7",
        ),
        &f.0,
        &f.0,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.exit_code, Some(7));
    assert!(result.stdout.contains("中文输出"), "{result:?}");
    assert!(f.0.join("cwd-marker.txt").is_file(), "{result:?}");
}
#[cfg(windows)]
#[test]
fn cancel_before_start_and_runtime_tampering_never_execute() {
    let f = Fixture::new();
    let mut s = spec(Language::Cmd, "echo unwanted>marker.txt");
    s.cwd = Some(f.0.to_string_lossy().into_owned());
    let result = run(&s, &f.0, &f.0, &AtomicBool::new(true)).unwrap();
    assert!(!result.ok);
    assert!(!f.0.join("marker.txt").exists());
    let data = json!({"script":s,"runtime":"C:/untrusted/python.exe"});
    let result = run_approved(
        &data,
        &s.code,
        s.cwd.as_deref().unwrap(),
        &f.0,
        &AtomicBool::new(false),
    );
    assert!(!result.ok);
    assert!(result.error.unwrap().contains("运行环境已变化"));
    assert!(!f.0.join("marker.txt").exists());
}
#[test]
#[ignore = "Run after prepare-python-runtime.ps1 -Destination rust/target/debug/runtime/python"]
fn bundled_python_batch_preview_apply_and_large_source() {
    let f = Fixture::new();
    let app = application_dir().unwrap();
    // 工作区里的同名模块绝不能遮住内置的标准库。
    fs::write(
        f.0.join("json.py"),
        "raise RuntimeError('workspace module executed')",
    )
    .unwrap();
    fs::write(
        f.0.join("样本.csv"),
        "name,status\n甲,open\n乙,open\n丙,done\n",
    )
    .unwrap();
    let code = r#"import csv, json, os, pathlib, hashlib, sys
data = json.loads(pathlib.Path(os.environ['MOCHI_SCRIPT_INPUT']).read_text(encoding='utf-8'))
source = pathlib.Path('样本.csv')
before = source.read_bytes()
rows = list(csv.DictReader(before.decode('utf-8').splitlines()))
matched = sum(row['status'] == 'open' for row in rows)
if os.environ['MOCHI_SCRIPT_INTENT'] == 'apply':
    if hashlib.sha256(before).hexdigest() != data['hash']:
        raise ValueError('conflict')
    with pathlib.Path('结果.json').open('w', encoding='utf-8') as output:
        json.dump(rows, output, ensure_ascii=False)
print(json.dumps({'matched':matched,'hash':hashlib.sha256(before).hexdigest(),'isolated':sys.flags.isolated,'value':data.get('value')},ensure_ascii=False))
"#;
    let mut s = spec(
        Language::Python,
        &format!(
            "#{}\n{code}",
            "large source not on command line ".repeat(1600)
        ),
    );
    s.intent = Intent::Preview;
    s.input = json!({"value":"中文 '&\" = no injection"});
    let result = run(&s, &f.0, &app, &AtomicBool::new(false)).unwrap();
    assert!(result.ok, "{result:?}");
    let output: Value = serde_json::from_str(result.stdout.trim()).unwrap();
    assert_eq!(output["matched"], 2);
    assert_eq!(output["isolated"], 1);
    assert_eq!(output["value"], s.input["value"]);
    assert!(!f.0.join("结果.json").exists());
    s.intent = Intent::Apply;
    s.input = json!({"hash":output["hash"]});
    let result = run(&s, &f.0, &app, &AtomicBool::new(false)).unwrap();
    assert!(result.ok, "{result:?}");
    assert!(fs::read_to_string(f.0.join("结果.json"))
        .unwrap()
        .contains("甲"));
    s.input = json!({"hash":"changed"});
    let result = run(&s, &f.0, &app, &AtomicBool::new(false)).unwrap();
    assert!(!result.ok);
    assert!(result.stderr.contains("conflict"));
}

#[cfg(windows)]
#[test]
#[ignore = "Requires bundled Python prepared next to Cargo profile"]
fn bundled_python_job_memory_limit_is_enforced() {
    let f = Fixture::new();
    let result = run(
        &spec(
            Language::Python,
            "print('allocation-test', flush=True)\nx = bytearray(700 * 1024 * 1024)",
        ),
        &f.0,
        &application_dir().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!result.ok, "allocation unexpectedly succeeded: {result:?}");
    assert!(result.stderr.contains("MemoryError"), "{result:?}");
    assert!(!result.timed_out);
}
#[test]
#[ignore = "Requires bundled Python prepared next to Cargo profile"]
fn bundled_python_cancellation_timeout_and_bounded_output() {
    let f = Fixture::new();
    let app = application_dir().unwrap();
    let mut s = spec(
        Language::Python,
        "import time\nprint('started', flush=True)\ntime.sleep(30)",
    );
    s.timeout_ms = 1000;
    let result = run(&s, &f.0, &app, &AtomicBool::new(false)).unwrap();
    assert!(result.timed_out);
    assert!(result.stdout.contains("started"));
    s.timeout_ms = 10000;
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(500));
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let result = run(&s, &f.0, &app, &cancel).unwrap();
        assert!(!result.ok);
        assert!(!result.timed_out);
        assert!(result.error.unwrap().contains("取消"));
    });
    let s = spec(
        Language::Python,
        "import sys\nprint('字' * 300000)\nsys.stderr.write('错' * 300000)",
    );
    let result = run(&s, &f.0, &app, &AtomicBool::new(false)).unwrap();
    assert!(result.ok);
    assert!(result.stdout.chars().count() < 201000);
    assert!(result.stderr.chars().count() < 201000);
}
