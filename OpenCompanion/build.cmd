@echo off
REM OpenCompanion CLI 一键构建（依赖本机 D:\DevEco Studio）
set PATH=D:\DevEco Studio\jbr\bin;D:\DevEco Studio\tools\node;D:\DevEco Studio\tools\ohpm\bin;%PATH%
set DEVECO_SDK_HOME=D:\DevEco Studio\sdk
cd /d %~dp0
node "D:\DevEco Studio\tools\hvigor\bin\hvigorw.js" --no-daemon -p product=default -p buildMode=debug assembleHap %*
echo.
echo 产物: entry\build\default\outputs\default\entry-default-unsigned.hap
