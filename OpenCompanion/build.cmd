@echo off
REM OpenCompanion CLI build (auto-detects DevEco Studio install path, no hard-coded drive)
REM Priority: DEVECO_HOME env var > common install locations
setlocal

if defined DEVECO_HOME if exist "%DEVECO_HOME%\tools\hvigor\bin\hvigorw.js" goto found
if defined DEVECO_HOME echo [build.cmd] DEVECO_HOME="%DEVECO_HOME%" is not a valid DevEco install, probing common locations...

for %%P in (
    "C:\Program Files\Huawei\DevEco Studio"
    "C:\Program Files (x86)\Huawei\DevEco Studio"
    "D:\DevEco Studio"
    "D:\Huawei\DevEco Studio"
    "E:\DevEco Studio"
    "C:\DevEco Studio"
) do (
    if exist "%%~P\tools\hvigor\bin\hvigorw.js" (
        set "DEVECO_HOME=%%~P"
        goto found
    )
)

echo [build.cmd] DevEco Studio not found.
echo Set the DEVECO_HOME env var to the install root and retry, e.g.:
echo   set DEVECO_HOME=C:\Program Files\Huawei\DevEco Studio
exit /b 1

:found
set "PATH=%DEVECO_HOME%\jbr\bin;%DEVECO_HOME%\tools\node;%DEVECO_HOME%\tools\ohpm\bin;%PATH%"
set "DEVECO_SDK_HOME=%DEVECO_HOME%\sdk"
cd /d %~dp0
echo [build.cmd] Using DevEco: %DEVECO_HOME%
node "%DEVECO_HOME%\tools\hvigor\bin\hvigorw.js" --no-daemon -p product=default -p buildMode=debug assembleHap %*
echo.
echo Output: entry\build\default\outputs\default\entry-default-unsigned.hap
endlocal
