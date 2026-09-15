#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <tlhelp32.h>
#include <wchar.h>

int main(void) {
    HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snapshot == INVALID_HANDLE_VALUE) return 1;
    PROCESSENTRY32W entry = {0};
    entry.dwSize = sizeof(entry);
    DWORD target = 0;
    DWORD group = 0;
    unsigned int matches = 0;
    if (Process32FirstW(snapshot, &entry)) {
        do {
            if (_wcsicmp(entry.szExeFile, L"VRisingServer.exe") == 0) {
                target = entry.th32ProcessID;
                group = entry.th32ParentProcessID;
                matches++;
            }
        } while (Process32NextW(snapshot, &entry));
    }
    BOOL launcher_found = FALSE;
    if (matches == 1 && Process32FirstW(snapshot, &entry)) {
        do {
            if (entry.th32ProcessID == group &&
                _wcsicmp(entry.szExeFile, L"helix-vrising-launch.exe") == 0) launcher_found = TRUE;
        } while (Process32NextW(snapshot, &entry));
    }
    CloseHandle(snapshot);
    if (matches != 1 || !launcher_found) return 2;
    FreeConsole();
    if (!AttachConsole(target)) return 3;
    if (!SetConsoleCtrlHandler(NULL, TRUE)) return 4;
    /* Wine 8 maps group zero to the caller's group, not all attached processes. */
    if (!GenerateConsoleCtrlEvent(CTRL_C_EVENT, group)) return 5;
    FreeConsole();
    return 0;
}
