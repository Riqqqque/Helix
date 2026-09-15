#define WIN32_LEAN_AND_MEAN
#include <windows.h>

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR command, int show) {
    (void)instance; (void)previous; (void)show;
    STARTUPINFOW startup = {0};
    PROCESS_INFORMATION process = {0};
    startup.cb = sizeof(startup);
    SetConsoleCtrlHandler(NULL, FALSE);
    if (!CreateProcessW(NULL, command, NULL, NULL, FALSE,
                        CREATE_NEW_CONSOLE,
                        NULL, NULL, &startup, &process)) return 1;
    CloseHandle(process.hThread);
    DWORD exit_code = 1;
    WaitForSingleObject(process.hProcess, INFINITE);
    GetExitCodeProcess(process.hProcess, &exit_code);
    CloseHandle(process.hProcess);
    return (int)exit_code;
}
