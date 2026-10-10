use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub found: bool,
    pub ffmpeg: Option<String>,
    pub ffprobe: Option<String>,
    pub version: Option<String>,
    pub whisper: bool,
    pub hardware_encoders: Vec<String>,
}

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// Creates a command that never flashes a console window on Windows.
pub fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.stdin(Stdio::null());
    command
}

fn candidate_dirs(configured: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_file() {
            if let Some(parent) = path.parent() {
                dirs.push(parent.to_path_buf());
            }
        } else {
            dirs.push(path.clone());
            dirs.push(path.join("bin"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("ffmpeg"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if cfg!(windows) {
        for base in ["C:\\Program Files\\ffmpeg\\bin", "C:\\ffmpeg\\bin", "C:\\Program Files (x86)\\ffmpeg\\bin"] {
            dirs.push(PathBuf::from(base));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(&local).join("Microsoft\\WinGet\\Links"));
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(&profile).join("scoop\\shims"));
        }
        dirs.push(PathBuf::from("C:\\ProgramData\\chocolatey\\bin"));
    } else {
        for base in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            dirs.push(PathBuf::from(base));
        }
    }
    dirs
}

pub fn locate(configured: &str) -> Option<Tools> {
    for dir in candidate_dirs(configured) {
        let ffmpeg = dir.join(format!("ffmpeg{EXE}"));
        let ffprobe = dir.join(format!("ffprobe{EXE}"));
        if ffmpeg.is_file() && ffprobe.is_file() {
            return Some(Tools { ffmpeg, ffprobe });
        }
    }
    None
}

pub fn require(configured: &str) -> AppResult<Tools> {
    locate(configured).ok_or_else(|| AppError::msg("FFmpeg was not found. Install FFmpeg or set its folder in Settings → Tools."))
}

pub fn status(configured: &str) -> ToolsStatus {
    let Some(tools) = locate(configured) else {
        return ToolsStatus { found: false, ffmpeg: None, ffprobe: None, version: None, whisper: false, hardware_encoders: vec![] };
    };
    let version = command(&tools.ffmpeg)
        .arg("-version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.lines().next().map(|line| line.trim().to_string()));
    let filters = command(&tools.ffmpeg)
        .args(["-hide_banner", "-filters"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let whisper = filters.lines().any(|line| line.split_whitespace().nth(1) == Some("whisper"));
    ToolsStatus {
        found: true,
        ffmpeg: Some(tools.ffmpeg.display().to_string()),
        ffprobe: Some(tools.ffprobe.display().to_string()),
        version,
        whisper,
        hardware_encoders: hardware_encoders(&tools),
    }
}

/// Hardware H.264 encoders that actually work on this machine (a quick trial encode each).
pub fn hardware_encoders(tools: &Tools) -> Vec<String> {
    ["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"]
        .into_iter()
        .filter(|encoder| {
            command(&tools.ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=256x256:d=0.2",
                    "-frames:v",
                    "3",
                    "-c:v",
                    encoder,
                    "-f",
                    "null",
                    "-",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
        .map(String::from)
        .collect()
}

/// Quotes a path for use as a filter option value: `'C\:/dir/file'`.
/// The graph parser keeps everything inside the quotes literally and the option
/// parser then unescapes `\:`.
pub fn filter_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/").replace(':', "\\:");
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Keeps the tail of FFmpeg's stderr for error messages.
pub fn stderr_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(6);
    lines[start..].join("\n")
}
