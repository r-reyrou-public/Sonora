@echo off
cd /d "%~dp0"
if exist Sonora.exe (
    start "" Sonora.exe
) else if exist target\release\sonora.exe (
    start "" target\release\sonora.exe
) else (
    echo Sonora.exe not found. Running cargo run --release...
    cargo run --release
)
