// No Taurscribe/GPU DLL imports: this must run even when the app cannot load.
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <wchar.h>
#include "launcher-files.h"
// Tauri stamps its public executable with the packaging format at bundle time.
static volatile const char bundle_type[] = "__TAURI_BUNDLE_TYPE_VAR_UNK";

static void repair_message(const wchar_t *detail) {
    wchar_t message[2048];
    _snwprintf_s(message, 2048, _TRUNCATE,
        L"Taurscribe needs repair.\n\n%ls\n\n"
        L"Run the Taurscribe installer again to restore the required files. "
        L"The online installer may need an internet connection.\n\n"
        L"Your settings, models, and recordings are stored separately from these program files.", detail);
    MessageBoxW(NULL, message, L"Taurscribe needs repair", MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
}

static int loader_error(DWORD code) {
    return code == 0xC0000135 || code == 0xC000007B || code == 0xC000012F ||
           code == 0xC0000139 || code == 0xC0000142;
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR arguments, int show) {
    (void)instance; (void)previous; (void)show;
    (void)bundle_type[0];
    wchar_t path[32768], detail[1024];
    DWORD length = GetModuleFileNameW(NULL, path, 32768);
    if (!length || length >= 32768) { repair_message(L"The application path could not be read."); return 1; }
    wchar_t *name = wcsrchr(path, L'\\');
    if (!name) return 1;
    ++name;
    size_t available = 32768 - (size_t)(name - path);
    for (size_t i = 0; i < sizeof(required_files) / sizeof(required_files[0]); ++i) {
        WIN32_FILE_ATTRIBUTE_DATA info;
        wcscpy_s(name, available, required_files[i].name);
        ULONGLONG size = 0;
        BOOL found = GetFileAttributesExW(path, GetFileExInfoStandard, &info);
        if (found) size = ((ULONGLONG)info.nFileSizeHigh << 32) | info.nFileSizeLow;
        if (!found || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || size != required_files[i].bytes) {
            _snwprintf_s(detail, 1024, _TRUNCATE, L"A required program file is missing or damaged:\n%ls", required_files[i].name);
            repair_message(detail);
            return 1;
        }
    }
    wcscpy_s(name, available, L"taurscribe-core.exe");
    size_t command_length = wcslen(path) + wcslen(arguments) + 5;
    wchar_t *command = HeapAlloc(GetProcessHeap(), 0, command_length * sizeof(wchar_t));
    if (!command) return 1;
    _snwprintf_s(command, command_length, _TRUNCATE, L"\"%ls\" %ls", path, arguments);

    // Keep console/MCP pipes, the caller's working directory, and exact arguments.
    AttachConsole(ATTACH_PARENT_PROCESS);
    STARTUPINFOW startup = { sizeof(startup) };
    PROCESS_INFORMATION process = { 0 };
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    startup.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
    startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    // Suppress the loader's generic DLL popup; show the actionable message below.
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    BOOL started = CreateProcessW(path, command, NULL, NULL, TRUE, 0, NULL, NULL, &startup, &process);
    DWORD error = GetLastError();
    HeapFree(GetProcessHeap(), 0, command);
    if (!started) {
        _snwprintf_s(detail, 1024, _TRUNCATE, L"Taurscribe could not start (Windows error %lu).", error);
        repair_message(detail);
        return 1;
    }
    CloseHandle(process.hThread);
    WaitForSingleObject(process.hProcess, INFINITE);
    DWORD code = 1;
    GetExitCodeProcess(process.hProcess, &code);
    CloseHandle(process.hProcess);
    if (loader_error(code)) {
        _snwprintf_s(detail, 1024, _TRUNCATE, L"Windows could not load a required program component (error 0x%08lX).", code);
        repair_message(detail);
        return 1;
    }
    return (int)code;
}
