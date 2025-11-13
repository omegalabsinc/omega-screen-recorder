@echo off
REM Screen Recording Conversion Script
REM This script converts the captured frames to a video file

set OUTPUT_FILE=..\demo.mp4
set CRF=18
set FFCONCAT_FILE=frames.ffconcat

if not exist %FFCONCAT_FILE% (
    echo Error: %FFCONCAT_FILE% not found
    exit /b 1
)

echo Converting frames to video...
echo Output: %OUTPUT_FILE%

where ffmpeg >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo Error: ffmpeg is not installed
    echo Download from: https://ffmpeg.org/download.html
    exit /b 1
)

ffmpeg -y -f concat -safe 0 -i %FFCONCAT_FILE% ^
    -vsync vfr -pix_fmt yuv420p ^
    -c:v libx264 -preset medium -crf %CRF% ^
    "%OUTPUT_FILE%"

if %ERRORLEVEL% EQU 0 (
    echo Video created: %OUTPUT_FILE%
) else (
    echo Conversion failed
    exit /b 1
)
