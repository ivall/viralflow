## Download

| System | File |
| --- | --- |
| Windows 10/11 (x64) | `ViralFlow-*-setup.exe` (installer) or `ViralFlow-*-windows-x64.zip` (portable) |
| macOS, Apple silicon | `ViralFlow-*-macos-arm64.dmg` |
| macOS, Intel | `ViralFlow-*-macos-x86_64.dmg` |
| Linux x64 | `ViralFlow-*-linux-x86_64.AppImage` or `.tar.gz` |
| Linux arm64 | `ViralFlow-*-linux-aarch64.AppImage` or `.tar.gz` |

FFmpeg is included in every download.

**macOS:** the app is not notarized yet. The first time, right-click ViralFlow in Applications and choose *Open* (or run `xattr -dr com.apple.quarantine /Applications/ViralFlow.app`).

**Linux:** make the AppImage executable (`chmod +x ViralFlow-*.AppImage`) and run it. Local Whisper captions need an FFmpeg built with whisper; the bundled Linux and macOS FFmpeg builds do not include it, so use cloud captions (OpenRouter) there.

Checksums are in `SHA256SUMS.txt`.
