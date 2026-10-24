@echo off
setlocal
cd /d "%~dp0"

echo ===================================================
echo Building Sonora (Release)...
echo ===================================================

cargo build --release
if errorlevel 1 (
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

copy /y target\release\sonora.exe .\Sonora.exe > nul

echo.
echo ===================================================
echo Build completed: Sonora.exe
echo ===================================================
pause
