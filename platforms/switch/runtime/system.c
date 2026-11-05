#include <switch.h>
#include <string.h>

/* Copies the system's Japan/US/Europe shared font (plain TrueType in pl shared
   memory) into `out`. Returns the font size, 0 when unavailable, or the required
   size without copying when `capacity` is too small. */
size_t cmvs_nx_shared_font(void *out, size_t capacity) {
    if (R_FAILED(plInitialize(PlServiceType_User)))
        return 0;
    PlFontData font = {0};
    size_t size = 0;
    if (R_SUCCEEDED(plGetSharedFontByType(&font, PlSharedFontType_Standard)) && font.address) {
        size = font.size;
        if (out && size <= capacity)
            memcpy(out, font.address, size);
    }
    plExit();
    return size;
}
