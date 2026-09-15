#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>

static HANDLE stopped;
static BOOL WINAPI on_control(DWORD event) {
    if (event != CTRL_C_EVENT) return FALSE;
    SetEvent(stopped);
    return TRUE;
}

int main(void) {
    stopped = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!stopped || !SetConsoleCtrlHandler(on_control, TRUE)) return 1;
    FILE *ready = fopen("test-ready", "w");
    if (!ready) return 2;
    fclose(ready);
    if (WaitForSingleObject(stopped, 120000) != WAIT_OBJECT_0) return 3;
    FILE *saved = fopen("test-saved", "w");
    if (!saved) return 4;
    fputs("saved", saved);
    fclose(saved);
    CloseHandle(stopped);
    return 0;
}
