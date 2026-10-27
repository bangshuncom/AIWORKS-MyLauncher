@echo off
chcp 65001 >nul 2>&1
title MyLauncher Portable Build

echo ============================================
echo   MyLauncher - Portable Build Script
echo ============================================
echo.

REM -------------------------------------------------------------
REM Ensure Rust environment is available even if the parent process
REM did not inherit the User PATH / RUSTUP_HOME / CARGO_HOME.
REM (e.g. double-clicking this .bat from a stale Explorer session)
REM -------------------------------------------------------------
if not defined RUSTUP_HOME set "RUSTUP_HOME=D:\Program Files\Rust\.rustup"
if not defined CARGO_HOME  set "CARGO_HOME=D:\Program Files\Rust\.cargo"
if "%PATH:D:\Program Files\Rust\.cargo\bin=%"=="%PATH%" set "PATH=D:\Program Files\Rust\.cargo\bin;%PATH%"

REM ?????? 1. Check Rust ??????
echo [1/4] Checking Rust toolchain...
where rustc >nul 2>&1
if errorlevel 1 (
    echo   [FAIL] Rust not found. Please install Rust first.
    echo   Download: https://rustup.rs
    pause
    exit /b 1
)
for /f "delims=" %%v in ('rustc --version') do echo   [OK] %%v

REM ?????? 2. Check Node.js ??????
echo [2/4] Checking Node.js...
where node >nul 2>&1
if errorlevel 1 (
    echo   [FAIL] Node.js not found. Please install Node.js first.
    echo   Download: https://nodejs.org
    pause
    exit /b 1
)
for /f "delims=" %%v in ('node --version') do echo   [OK] Node.js %%v
for /f "delims=" %%v in ('npm --version') do echo   [OK] npm %%v

REM ?????? 3. Check node_modules ??????
echo [3/4] Checking dependencies...
if not exist "MyLauncher-src\node_modules" (
    echo   node_modules not found, installing...
    cd MyLauncher-src
    npm install
    cd ..
)

REM ?????? 4. Build and deploy ??????
echo [4/4] Building Tauri app...
cd MyLauncher-src
call npm run tauri build
if errorlevel 1 (
    echo.
    echo   [FAIL] Build failed! Check error output above.
    pause
    exit /b 1
)
echo   [OK] Build succeeded
cd ..

REM ?????? Deploy to MyLauncherPortable ??????
echo.
echo   Deploying to MyLauncherPortable...

set "EXE_PATH=MyLauncher-src\src-tauri\target\release\MyLauncher.exe"
set "PKG_DIR=%CD%\MyLauncherPortable"

if not exist "%EXE_PATH%" (
    echo   [FAIL] exe not found: %EXE_PATH%
    pause
    exit /b 1
)

REM Only update exe, keep existing data/assets
copy /Y "%EXE_PATH%" "%PKG_DIR%\MyLauncher.exe" >nul
echo   [OK] exe updated: %PKG_DIR%\MyLauncher.exe

REM Ensure directory structure exists (create on first deploy, skip if exists)
if not exist "%PKG_DIR%\data" mkdir "%PKG_DIR%\data"
if not exist "%PKG_DIR%\assets\icons\custom" mkdir "%PKG_DIR%\assets\icons\custom"
if not exist "%PKG_DIR%\assets\icons\extracted" mkdir "%PKG_DIR%\assets\icons\extracted"
if not exist "%PKG_DIR%\assets\icons\builtin" mkdir "%PKG_DIR%\assets\icons\builtin"
if not exist "%PKG_DIR%\assets\covers\custom" mkdir "%PKG_DIR%\assets\covers\custom"
if not exist "%PKG_DIR%\assets\covers\thumbs" mkdir "%PKG_DIR%\assets\covers\thumbs"
if not exist "%PKG_DIR%\assets\temp" mkdir "%PKG_DIR%\assets\temp"

REM Deploy scan plugins (plugins/ folder, xcopy keeps subdirs per plugin)
if exist "%CD%\plugins" (
    xcopy "%CD%\plugins" "%PKG_DIR%\plugins\" /E /I /Y >nul
    echo   [OK] plugins deployed
)

echo.
echo ============================================
echo   Build complete!
echo   Output: %PKG_DIR%
echo   Run MyLauncher.exe to start
echo ============================================
pause
