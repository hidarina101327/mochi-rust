//! CPU 用 GetProcessTimes 差分采样；首次没有基线，返回 None。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessTimes};

/// 一次采样的结果。每个字段独立可用；某个 API 失败不会用 0 冒充结果。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Snapshot {
    pub(super) cpu_percent: Option<f32>,
    pub(super) working_set_bytes: Option<u64>,
    pub(super) private_bytes: Option<u64>,
    pub(super) thread_count: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct CpuSample {
    process_ticks: u64,
    at: Instant,
}

/// 负责给状态栏提供本进程的低频资源快照。
#[derive(Debug)]
pub(super) struct Sampler {
    latest: Option<Snapshot>,
    previous_cpu: Option<CpuSample>,
    logical_processors: u32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub(super) fn new() -> Self {
        let logical_processors = std::thread::available_parallelism()
            .map(|value| value.get() as u32)
            .unwrap_or(1)
            .max(1);
        Self {
            latest: None,
            previous_cpu: None,
            logical_processors,
        }
    }

    /// 读取一次当前进程的资源状态。
    ///
    /// 内存和线程计数可以在第一次调用时得到；CPU 是累计进程时间的差分，
    /// 所以在第一次成功读取时间后保持未知，直到下一次成功读取。
    pub(super) fn sample(&mut self) -> Snapshot {
        let now = Instant::now();
        let process = unsafe { GetCurrentProcess() };
        let (working_set_bytes, private_bytes) = process_memory(process);
        let cpu_percent = self.cpu_percent(process, now);
        let thread_count = process_thread_count(unsafe { GetCurrentProcessId() });
        let snapshot = Snapshot {
            cpu_percent,
            working_set_bytes,
            private_bytes,
            thread_count,
        };
        self.latest = Some(snapshot);
        snapshot
    }

    pub(super) fn snapshot(&self) -> Option<Snapshot> {
        self.latest.filter(|snapshot| {
            snapshot.cpu_percent.is_some()
                || snapshot.working_set_bytes.is_some()
                || snapshot.private_bytes.is_some()
                || snapshot.thread_count.is_some()
        })
    }

    fn cpu_percent(
        &mut self,
        process: windows::Win32::Foundation::HANDLE,
        now: Instant,
    ) -> Option<f32> {
        let process_ticks = process_cpu_ticks(process)?;
        let result = self.previous_cpu.and_then(|previous| {
            let elapsed = now.checked_duration_since(previous.at).unwrap_or_default();
            cpu_percent_from_delta(
                process_ticks.saturating_sub(previous.process_ticks),
                elapsed,
                self.logical_processors,
            )
        });
        // API 成功时更新基线；短暂失败时保留旧基线，让下一次成功采样仍可计算差分。
        self.previous_cpu = Some(CpuSample {
            process_ticks,
            at: now,
        });
        result
    }
}

fn process_memory(process: windows::Win32::Foundation::HANDLE) -> (Option<u64>, Option<u64>) {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            process,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    };
    if ok.is_err() {
        return (None, None);
    }
    (
        Some(counters.WorkingSetSize as u64),
        Some(counters.PrivateUsage as u64),
    )
}

fn process_cpu_ticks(process: windows::Win32::Foundation::HANDLE) -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
    }
    Some(filetime_ticks(kernel).saturating_add(filetime_ticks(user)))
}

fn filetime_ticks(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn process_thread_count(process_id: u32) -> Option<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.ok()?;

    unsafe {
        let result = (|| {
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            Thread32First(snapshot, &mut entry).ok()?;
            let mut count = 0u32;
            loop {
                if entry.th32OwnerProcessID == process_id {
                    count = count.saturating_add(1);
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
            // 进程至少有一个线程；0 通常意味着枚举失败或快照竞争，保持未知。
            (count > 0).then_some(count)
        })();
        let _ = CloseHandle(snapshot);
        result
    }
}

fn cpu_percent_from_delta(
    process_ticks: u64,
    elapsed: Duration,
    logical_processors: u32,
) -> Option<f32> {
    if elapsed.is_zero() || logical_processors == 0 {
        return None;
    }
    // FILETIME 是 100ns 单位；按逻辑处理器数归一化，范围为 0..=100%。
    let process_seconds = process_ticks as f64 / 10_000_000.0;
    let wall_seconds = elapsed.as_secs_f64();
    Some(
        (process_seconds / wall_seconds / f64::from(logical_processors) * 100.0).clamp(0.0, 100.0)
            as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_cpu_sample_is_unknown_until_a_delta_exists() {
        assert_eq!(cpu_percent_from_delta(10_000_000, Duration::ZERO, 8), None);
    }

    #[test]
    fn cpu_delta_is_normalized_by_logical_processors() {
        let value = cpu_percent_from_delta(10_000_000, Duration::from_secs(2), 4);
        assert_eq!(value, Some(12.5));
    }

    #[test]
    fn cpu_delta_is_clamped_when_a_short_interval_rounds_up() {
        let value = cpu_percent_from_delta(80_000_000, Duration::from_secs(1), 1);
        assert_eq!(value, Some(100.0));
    }
}
