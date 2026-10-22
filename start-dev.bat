@echo off
setlocal
cd /d E:\cc-router-main

echo ============================================
echo   cc-router dev launcher
echo ============================================
echo.

where pnpm >nul 2>&1
if errorlevel 1 (
    echo [ERROR] pnpm not found. Run: npm i -g pnpm
    pause
    exit /b 1
)

if not exist node_modules (
    echo [INFO] node_modules missing, running pnpm install...
    call pnpm install
    if errorlevel 1 (
        echo [ERROR] pnpm install failed.
        pause
        exit /b 1
    )
)

echo [INFO] Starting cc-router (tauri dev)...
echo [INFO] Frontend: http://127.0.0.1:1420
echo [INFO] Proxy:     http://127.0.0.1:23456
echo.
call pnpm tauri:dev

echo.
echo [INFO] cc-router exited.
pause