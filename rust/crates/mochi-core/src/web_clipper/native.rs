//! 注册 Windows 原生消息宿主，并通过仅限当前用户访问的命名管道通信。
use super::*;
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::{
        io::{AsRawHandle, FromRawHandle},
        process::CommandExt,
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Pipes::*, Threading::*},
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn sid() -> Result<String> {
    process_sid(unsafe { GetCurrentProcess() })
}
fn process_sid(process: HANDLE) -> Result<String> {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut bytes = vec![0usize; (size as usize + 7) / 8];
        if GetTokenInformation(token, TokenUser, bytes.as_mut_ptr().cast(), size, &mut size) == 0 {
            CloseHandle(token);
            return Err(std::io::Error::last_os_error().into());
        }
        let user = &*(bytes.as_ptr() as *const TOKEN_USER);
        let mut text = std::ptr::null_mut();
        let ok = ConvertSidToStringSidW(user.User.Sid, &mut text);
        CloseHandle(token);
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        Ok(value)
    }
}
fn pipe_name() -> Result<String> {
    let scope = std::env::var_os("MOCHI_SETTINGS_FILE")
        .or_else(|| std::env::var_os("MOCHI_SETTINGS_PATH"))
        .map(|p| workspace_token(Path::new(&p)))
        .unwrap_or_default();
    Ok(format!(
        r"\\.\pipe\Mochi.WebClipper.v1.{}.{}",
        sid()?,
        scope
    ))
}
fn verify_server(pipe: &File) -> Result<()> {
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let owner = process_sid(process);
        CloseHandle(process);
        if owner? != sid()? {
            bail!("本地接收端不是当前用户的进程");
        }
    }
    Ok(())
}
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Value>> {
    let mut length = [0; 4];
    match reader.read(&mut length[..1]) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(e) => return Err(e.into()),
    }
    reader.read_exact(&mut length[1..])?;
    let len = u32::from_le_bytes(length) as usize;
    if len == 0 || len > MAX_FRAME {
        bail!("消息大小超出限制");
    }
    let mut data = vec![0; len];
    reader.read_exact(&mut data)?;
    Ok(Some(serde_json::from_slice(&data)?))
}
pub fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME {
        bail!("消息大小超出限制");
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
pub fn serve(receiver: Arc<Receiver>) -> Result<()> {
    let name = pipe_name()?;
    let security = wide(&format!("D:P(A;;GA;;;{})", sid()?));
    std::thread::Builder::new()
        .name("web-clipper-pipe".into())
        .spawn(move || loop {
            let handle = unsafe {
                let mut descriptor = std::ptr::null_mut();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    security.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                ) == 0
                {
                    return;
                }
                let attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor,
                    bInheritHandle: 0,
                };
                let h = CreateNamedPipeW(
                    wide(&name).as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    8,
                    MAX_FRAME as u32,
                    MAX_FRAME as u32,
                    0,
                    &attributes,
                );
                LocalFree(descriptor);
                h
            };
            if handle == INVALID_HANDLE_VALUE {
                return;
            }
            let connected = unsafe {
                ConnectNamedPipe(handle, std::ptr::null_mut()) != 0
                    || GetLastError() == ERROR_PIPE_CONNECTED
            };
            if !connected {
                unsafe {
                    CloseHandle(handle);
                }
                continue;
            }
            let receiver = Arc::clone(&receiver);
            let mut file = unsafe { File::from_raw_handle(handle) };
            std::thread::spawn(move || {
                while let Ok(Some(envelope)) = read_frame(&mut file) {
                    let origin = envelope["origin"].as_str().unwrap_or("");
                    let reply = receiver.handle(origin, envelope["request"].clone());
                    if write_frame(&mut file, &reply).is_err() {
                        break;
                    }
                }
            });
        })?;
    Ok(())
}
pub fn receiver_running() -> bool {
    let Ok(name) = pipe_name() else { return false };
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .is_ok_and(|pipe| verify_server(&pipe).is_ok())
}

pub fn host_main() -> Result<()> {
    let origin = std::env::args()
        .nth(1)
        .context("必须由浏览器启动本地宿主")?;
    let settings = SettingsService::new(None);
    if !allowed(&settings, &origin) {
        bail!("扩展未绑定");
    }
    let pipe_name = pipe_name()?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(request) = read_frame(&mut input)? {
        let result = (|| -> Result<Value> {
            let connect = || {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&pipe_name)
            };
            let mut pipe = match connect() {
                Ok(p) => p,
                Err(_) => {
                    let parent = std::env::current_exe()?
                        .parent()
                        .context("找不到安装目录")?
                        .to_path_buf();
                    let app = if parent.join("Mochi.exe").is_file() {
                        parent.join("Mochi.exe")
                    } else {
                        parent.join("mochi-app.exe")
                    };
                    Command::new(app)
                        .arg("--web-clipper")
                        .creation_flags(0x08000000)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?;
                    let start = Instant::now();
                    loop {
                        if let Ok(p) = connect() {
                            break p;
                        }
                        if start.elapsed() > Duration::from_secs(30) {
                            bail!("墨池启动超时，请打开墨池后重试");
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            };
            verify_server(&pipe)?;
            write_frame(&mut pipe, &json!({"origin":origin,"request":request}))?;
            read_frame(&mut pipe)?.context("墨池接收端已断开")
        })();
        let reply = result.unwrap_or_else(
            |e| json!({"version":VERSION,"id":request["id"],"ok":false,"error":format!("{e:#}")}),
        );
        write_frame(&mut output, &reply)?;
    }
    Ok(())
}

pub fn register(settings: &SettingsService) -> Result<()> {
    let base = std::env::var_os("LOCALAPPDATA").context("缺少 LOCALAPPDATA")?;
    let dir = PathBuf::from(base).join("Mochi/NativeMessaging");
    std::fs::create_dir_all(&dir)?;
    let _guard = crate::settings::file::Lock::acquire(&dir.join("registration"))?;
    settings.reload()?;
    // 必须在浏览器完成首次配对前创建启动器。
    register_launcher()?;
    let enabled = settings.get("webClipper.enabled").as_deref() == Some("true");
    let ids = extension_ids(settings)?;
    let path = dir.join(format!("{HOST}.json"));
    if enabled && !ids.is_empty() {
        let executable = std::env::current_exe()?
            .parent()
            .context("找不到安装目录")?
            .join("mochi-clipper-host.exe");
        if !executable.is_file() {
            bail!("缺少 mochi-clipper-host.exe，请构建桥接程序或重新安装墨池");
        }
        crate::files::FileService::new().write_file_safe(&path,&serde_json::to_string_pretty(&json!({"name":HOST,"description":"墨池网页剪藏本地接收端","path":executable,"type":"stdio","allowed_origins":ids.iter().map(|id|format!("chrome-extension://{id}/")).collect::<Vec<_>>()}))?)?;
    }
    for browser in ["Google\\Chrome", "Microsoft\\Edge"] {
        let key = format!("HKCU\\Software\\{browser}\\NativeMessagingHosts\\{HOST}");
        let system = std::env::var_os("SystemRoot").context("缺少 SystemRoot")?;
        let mut command = Command::new(PathBuf::from(system).join("System32/reg.exe"));
        if enabled && !ids.is_empty() {
            command
                .args(["add", &key, "/ve", "/t", "REG_SZ", "/d"])
                .arg(&path)
                .arg("/f");
        } else {
            command.args(["delete", &key, "/f"]);
        }
        let status = command
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if enabled && !ids.is_empty() && !status.success() {
            bail!("注册本地接收端失败");
        }
    }
    Ok(())
}

fn register_launcher() -> Result<()> {
    let executable = std::env::current_exe()?;
    let parent = executable.parent().context("找不到安装目录")?;
    let app = if parent.join("Mochi.exe").is_file() {
        parent.join("Mochi.exe")
    } else {
        parent.join("mochi-app.exe")
    };
    if !app.is_file() {
        bail!("找不到墨池客户端，请重新安装墨池");
    }
    let system = std::env::var_os("SystemRoot").context("缺少 SystemRoot")?;
    let command = format!("\"{}\" --connect-web-clipper \"%1\"", app.display());
    for (key, name, value) in [
        (
            r"HKCU\Software\Classes\mochi-clipper",
            None,
            "URL:墨池网页剪藏",
        ),
        (
            r"HKCU\Software\Classes\mochi-clipper",
            Some("URL Protocol"),
            "",
        ),
        (
            r"HKCU\Software\Classes\mochi-clipper\shell\open\command",
            None,
            command.as_str(),
        ),
    ] {
        let mut reg = Command::new(PathBuf::from(&system).join("System32/reg.exe"));
        reg.args(["add", key]);
        if let Some(name) = name {
            reg.args(["/v", name]);
        } else {
            reg.arg("/ve");
        }
        if !reg
            .args(["/t", "REG_SZ", "/d", value, "/f"])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success()
        {
            bail!("注册墨池启动入口失败");
        }
    }
    Ok(())
}
