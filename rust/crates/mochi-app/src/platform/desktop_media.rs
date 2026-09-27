//! Windows System Media Transport Controls (GSMTC) integration.
//!
//! This module only reads sessions unless a caller explicitly invokes
//! [`control`]. `preferred_source` is a GSMTC source app user-model ID. An
//! empty value follows the system's current session; a non-empty value targets
//! that session without changing Windows' system-wide current-session choice.

use anyhow::{anyhow, bail, Context, Result};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::{
    RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED,
};

const TICKS_PER_SECOND: i64 = 10_000_000;

/// The active media session, if one is available, plus all discovered sources.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub title: String,
    pub artist: String,
    /// GSMTC source app user-model ID. Persist this value for source selection.
    pub source: String,
    pub playing: bool,
    pub position_seconds: u64,
    pub duration_seconds: u64,
    /// Pairs of stable source ID and display label.
    pub sources: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Previous,
    Next,
    /// Position relative to the beginning of the current track.
    SeekSeconds(u64),
}

struct WinRtApartment;

impl WinRtApartment {
    fn initialize() -> Result<Self> {
        // Match the process' existing COM apartment if a host thread has
        // already initialized COM. A mismatched first attempt does not need a
        // corresponding uninitialize; the successful retry does.
        match unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
            Ok(()) => Ok(Self),
            Err(error) if error.code() == RPC_E_CHANGED_MODE => {
                unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                    .map_err(|error| anyhow!(error).context("初始化 Windows Runtime 失败"))?;
                Ok(Self)
            }
            Err(error) => Err(anyhow!(error).context("初始化 Windows Runtime 失败")),
        }
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn media_manager() -> Result<Manager> {
    Manager::RequestAsync()
        .context("请求系统媒体会话管理器失败")?
        .join()
        .context("等待系统媒体会话管理器失败")
}

fn sessions(manager: &Manager) -> Result<Vec<Session>> {
    let view = manager.GetSessions().context("枚举系统媒体会话失败")?;
    let count = view.Size().context("读取系统媒体会话数量失败")?;
    let mut sessions = Vec::with_capacity(count as usize);
    for index in 0..count {
        sessions.push(
            view.GetAt(index)
                .with_context(|| format!("读取系统媒体会话 {index} 失败"))?,
        );
    }
    Ok(sessions)
}

fn source_id(session: &Session) -> String {
    session
        .SourceAppUserModelId()
        .map(|id| id.to_string())
        .unwrap_or_default()
}

fn source_label(id: &str) -> String {
    let candidate = id.rsplit_once('!').map_or(id, |(_, app)| app);
    if candidate.is_empty() {
        id.to_owned()
    } else {
        candidate.to_owned()
    }
}

fn seconds(ticks: i64) -> u64 {
    u64::try_from(ticks.max(0) / TICKS_PER_SECOND).unwrap_or(0)
}

fn select_session(
    manager: &Manager,
    sessions: &[Session],
    preferred_source: &str,
) -> Option<Session> {
    let preferred_source = preferred_source.trim();
    if !preferred_source.is_empty() {
        return sessions
            .iter()
            .find(|session| source_id(session).eq_ignore_ascii_case(preferred_source))
            .cloned();
    }
    manager
        .GetCurrentSession()
        .ok()
        .or_else(|| sessions.first().cloned())
}

/// Read available system media sessions and current track metadata.
///
/// With no available session (or if a persisted preferred source has gone
/// away), returns an empty snapshot and the discovered source list.
pub fn snapshot(preferred_source: &str) -> Result<Snapshot> {
    let _apartment = WinRtApartment::initialize()?;
    let manager = media_manager()?;
    let sessions = sessions(&manager)?;
    let sources = sessions
        .iter()
        .map(|session| {
            let id = source_id(session);
            (id.clone(), source_label(&id))
        })
        .filter(|(id, _)| !id.is_empty())
        .collect::<Vec<_>>();
    let Some(session) = select_session(&manager, &sessions, preferred_source) else {
        return Ok(Snapshot {
            sources,
            ..Snapshot::default()
        });
    };

    let source = source_id(&session);
    let properties = session
        .TryGetMediaPropertiesAsync()
        .context("请求当前媒体信息失败")?
        .join()
        .context("读取当前媒体信息失败")?;
    let timeline = session
        .GetTimelineProperties()
        .context("读取媒体播放进度失败")?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let position = timeline.Position()?.Duration;
    let duration_seconds = seconds(end.saturating_sub(start));
    let position_seconds = seconds(position.saturating_sub(start)).min(duration_seconds);
    let playing = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|status| status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);

    Ok(Snapshot {
        available: true,
        title: properties.Title()?.to_string(),
        artist: properties.Artist()?.to_string(),
        source,
        playing,
        position_seconds,
        duration_seconds,
        sources,
    })
}

/// Send one user-requested command to the selected media session.
pub fn control(preferred_source: &str, command: Command) -> Result<()> {
    let _apartment = WinRtApartment::initialize()?;
    let manager = media_manager()?;
    let sessions = sessions(&manager)?;
    let Some(session) = select_session(&manager, &sessions, preferred_source) else {
        bail!("没有可控制的媒体会话");
    };

    let accepted = match command {
        Command::PlayPause => session
            .TryTogglePlayPauseAsync()
            .context("请求切换播放状态失败")?
            .join()
            .context("等待播放状态切换失败")?,
        Command::Previous => session
            .TrySkipPreviousAsync()
            .context("请求上一首失败")?
            .join()
            .context("等待上一首操作失败")?,
        Command::Next => session
            .TrySkipNextAsync()
            .context("请求下一首失败")?
            .join()
            .context("等待下一首操作失败")?,
        Command::SeekSeconds(requested_seconds) => {
            let timeline = session
                .GetTimelineProperties()
                .context("读取媒体时间轴失败")?;
            let start = timeline.StartTime()?.Duration;
            let end = timeline.EndTime()?.Duration;
            let minimum = timeline.MinSeekTime()?.Duration.max(start);
            let maximum = timeline.MaxSeekTime()?.Duration;
            let maximum = if maximum > 0 {
                maximum.min(end.max(start))
            } else {
                end.max(start)
            };
            let requested = start.saturating_add(
                i64::try_from(requested_seconds)
                    .unwrap_or(i64::MAX)
                    .saturating_mul(TICKS_PER_SECOND),
            );
            let requested = requested.clamp(minimum.min(maximum), maximum.max(minimum));
            session
                .TryChangePlaybackPositionAsync(requested)
                .context("请求调整播放进度失败")?
                .join()
                .context("等待播放进度调整失败")?
        }
    };
    if !accepted {
        return Err(anyhow!("媒体应用拒绝了此操作"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{seconds, source_label, TICKS_PER_SECOND};

    #[test]
    fn source_labels_keep_aumid_as_the_stable_selection_key() {
        assert_eq!(source_label("Contoso.Player_1!Main"), "Main");
        assert_eq!(source_label("Legacy.Player"), "Legacy.Player");
    }

    #[test]
    fn time_spans_are_nonnegative_and_report_whole_seconds() {
        assert_eq!(seconds(-1), 0);
        assert_eq!(seconds(0), 0);
        assert_eq!(seconds(TICKS_PER_SECOND * 65 + 9), 65);
    }

    #[test]
    #[ignore = "reads real Windows media sessions without sending playback commands"]
    fn system_media_read_only_smoke() {
        let current = super::snapshot("").expect("read Windows GSMTC session manager");
        println!(
            "GSMTC available: {}; source count: {}",
            current.available,
            current.sources.len()
        );
        assert!(current.position_seconds <= current.duration_seconds);
    }
}
