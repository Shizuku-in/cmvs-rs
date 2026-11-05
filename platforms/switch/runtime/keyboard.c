#include <switch.h>
#include <stddef.h>

/* Validates the UTF-8 `text` on OK. Returns 0 to accept; otherwise writes a NUL-terminated
   message of at most `capacity` bytes into `message` and the keyboard stays open. */
typedef int (*CmvsTextCheck)(const char *text, char *message, size_t capacity, void *user);

/* swkbd runs modally on the calling thread; its callback has no user pointer. */
static CmvsTextCheck active_check;
static void *active_user;

static SwkbdTextCheckResult check_text(char *text, size_t size) {
    /* The callback buffer holds the input and receives the message in place. */
    char input[1024];
    size_t length = 0;
    while (length + 1 < sizeof(input) && length < size && text[length]) {
        input[length] = text[length];
        ++length;
    }
    input[length] = 0;
    return active_check(input, text, size, active_user) == 0 ? SwkbdTextCheckResult_OK
                                                             : SwkbdTextCheckResult_Bad;
}

/* Shows the system keyboard. Returns 1 with UTF-8 in `out`, 0 when cancelled, -1 on
   failure. `out` must hold the check callback's input too (swkbd shares the buffer). */
int cmvs_nx_keyboard(const char *header, const char *sub, const char *initial, u32 max_chars,
                     CmvsTextCheck check, void *user, char *out, size_t out_size) {
    SwkbdConfig keyboard;
    if (R_FAILED(swkbdCreate(&keyboard, 0)))
        return -1;
    swkbdConfigMakePresetDefault(&keyboard);
    swkbdConfigSetHeaderText(&keyboard, header);
    swkbdConfigSetSubText(&keyboard, sub);
    swkbdConfigSetInitialText(&keyboard, initial);
    swkbdConfigSetStringLenMax(&keyboard, max_chars);
    active_check = check;
    active_user = user;
    swkbdConfigSetTextCheckCallback(&keyboard, check_text);
    Result rc = swkbdShow(&keyboard, out, out_size);
    active_check = NULL;
    active_user = NULL;
    swkbdClose(&keyboard);
    if (rc == MAKERESULT(Module_Libnx, LibnxError_LibAppletBadExit))
        return 0;
    return R_SUCCEEDED(rc) ? 1 : -1;
}
