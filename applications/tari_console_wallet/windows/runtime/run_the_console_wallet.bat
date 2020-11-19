@echo off
rem Verify arguments
if ["%config_path%"]==[""] (
    echo Problem with "config_path" environment variable: '%config_path%'
    pause
    exit /b 10101
)
if not exist "%config_path%" (
    echo Path as per "config_path" environment variable not found: '%config_path%'
    pause
    exit /b 10101
)
if ["%base_path%"]==[""] (
    echo Problem with "base_path" environment variable: '%base_path%'
    pause
    exit /b 10101
)
if not exist "%base_path%" (
    echo Path as per "base_path" environment variable not found: '%base_path%'
    pause
    exit /b 10101
)
if ["%my_exe%"]==[""] (
    echo Problem with "my_exe" environment variable: '%my_exe%'
    pause
    exit /b 10101
)
if ["%sqlite_runtime%"]==[""] (
    echo Problem with "sqlite_runtime" environment variable: '%sqlite_runtime%'
    pause
    exit /b 10101
)

rem Verify SQLite's location and prepend the default location to the system path if it exist
if exist "%TARI_SQLITE_DIR%\%sqlite_runtime%" (
    set "path=%TARI_SQLITE_DIR%;%path%"
    echo.
    echo Default location of "%sqlite_runtime%" prepended to the system path
) else if exist "%USERPROFILE%\.sqlite\%sqlite_runtime%" (
    set "path=%USERPROFILE%\.sqlite;%path%"
    echo.
    echo Default location of "%sqlite_runtime%" prepended to the system path
) else (
    set FOUND=
    for %%X in (%sqlite_runtime%) do (set FOUND=%%~$PATH:X)
    if defined FOUND (
        echo.
        echo "%sqlite_runtime%" found in system path:
        where "%sqlite_runtime%"
    ) else (
        echo.
        echo Note: "%sqlite_runtime%" not found in the default location or in the system path; this may be a problem
		if ["%TARI_SQLITE_DIR%"]==["%USERPROFILE%\.sqlite"] (
			echo       {default location: tried "%TARI_SQLITE_DIR%"}
		) else (
			echo       {default location: tried "%TARI_SQLITE_DIR%" and "%USERPROFILE%\.sqlite"}
		)
        echo.
        pause
    )
)

rem Find the console wallet executable
if exist "%my_exe_path%\%my_exe%" (
    set console_wallet=%my_exe_path%\%my_exe%
    echo.
    echo Using "%my_exe%" found in my_exe_path
    echo.
) else (
    if exist "%base_path%\%my_exe%" (
        set console_wallet=%base_path%\%my_exe%
        echo.
        echo Using "%my_exe%" found in base_path
        echo.
    ) else (
        set FOUND=
        for %%X in (%my_exe%) do (set FOUND=%%~$PATH:X)
        if defined FOUND (
            set console_wallet=%my_exe%
            echo.
            echo Using "%my_exe%" found in system path:
            where "%my_exe%"
            echo.
        ) else (
            echo.
            echo Runtime "%my_exe%" not found in my_exe_path, base_path or the system path
            echo.
            pause
            exit /b 10101
        )
    )
)

rem First time run
if not exist "%config_path%\console_wallet_id.json" (
    "%console_wallet%" --create-id --config "%config_path%\windows.toml" --log_config "%config_path%\log4rs_console_wallet.yml" --base-path "%base_path%"
    echo.
    echo.
    echo Created "%config_path%\console_wallet_id.json".
    echo.
) else (
    echo.
    echo.
    echo Using old "%config_path%\console_wallet_id.json"
    echo.
)

rem Consecutive runs
"%console_wallet%" --config "%config_path%\windows.toml" --log_config "%config_path%\log4rs_console_wallet.yml" --base-path "%base_path%"
