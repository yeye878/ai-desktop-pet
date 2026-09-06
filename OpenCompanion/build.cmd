@echo off
REM OpenCompanion CLI 一键构建（适配本机 C:\Program Files\Huawei\DevEco Studio）
set PATH=C:\Program Files\Huawei\DevEco Studio\jbr\bin;C:\Program Files\Huawei\DevEco Studio\tools\node;C:\Program Files\Huawei\DevEco Studio\tools\ohpm\bin;%PATH%
set DEVECO_SDK_HOME=C:\Program Files\Huawei\DevEco Studio\sdk
cd /d %~dp0
node "C:\Program Files\Huawei\DevEco Studio\tools\hvigor\bin\hvigorw.js" --no-daemon -p product=default -p buildMode=debug assembleHap %*
echo.
echo 产物: entry\build\default\outputs\default\entry-default-unsigned.hap
