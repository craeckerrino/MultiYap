@echo off
setlocal
chcp 65001 >nul
title MultiYap
cd /d "%~dp0"

where node >nul 2>nul
if errorlevel 1 (
  echo Node.js wurde nicht gefunden. Bitte Node.js installieren.
  goto fehler
)

where cargo >nul 2>nul
if errorlevel 1 (
  if exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
    set "PATH=%PATH%;%USERPROFILE%\.cargo\bin"
  ) else (
    echo Rust wurde nicht gefunden. Bitte Rust installieren.
    goto fehler
  )
)

if not exist "node_modules\" (
  echo Abhängigkeiten werden installiert ...
  call npm ci --no-fund --no-audit
  if errorlevel 1 goto fehler
)

echo MultiYap wird gebaut ...
call npm run build --silent
if errorlevel 1 (
  if not exist "dist\index.html" goto fehler
  echo.
  echo Der Build ist fehlgeschlagen. Der letzte funktionierende Stand wird gestartet.
)

echo Der lokale Server wird gebaut ...
cargo build --release --quiet -p multiyap-server
if errorlevel 1 (
  if not exist "target\release\multiyap-server.exe" goto fehler
  echo.
  echo Der Server konnte nicht neu gebaut werden. Der letzte funktionierende Stand wird gestartet.
)

echo.
"target\release\multiyap-server.exe" %*
if errorlevel 1 goto fehler
exit /b 0

:fehler
echo.
echo MultiYap konnte nicht gestartet werden.
pause
exit /b 1
