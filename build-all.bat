@echo off
REM One-click full build: CA -> setup -> sign -> hash.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-all.ps1"
echo.
pause