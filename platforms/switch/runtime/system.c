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

/* FastLoad boost: CPU 1785 MHz, GPU throttled to minimum. Fails outside application
   mode or before 7.0.0. Returns 0 on success. */
int cmvs_nx_set_cpu_boost(int enabled) {
    Result rc = appletSetCpuBoostMode(enabled ? ApmCpuBoostMode_FastLoad : ApmCpuBoostMode_Normal);
    return R_SUCCEEDED(rc) ? 0 : -1;
}

/* Shows the error applet, blocking until dismissed. The application variant (the one
   games use) is fatal outside application mode, where the system variant stands in.
   Returns 0 when it was shown. */
int cmvs_nx_show_error(const char *dialog, const char *details) {
    Result rc;
    if (appletGetAppletType() == AppletType_Application) {
        ErrorApplicationConfig config;
        rc = errorApplicationCreate(&config, dialog, details);
        if (R_SUCCEEDED(rc))
            rc = errorApplicationShow(&config);
    } else {
        ErrorSystemConfig config;
        rc = errorSystemCreate(&config, dialog, details);
        if (R_SUCCEEDED(rc))
            rc = errorSystemShow(&config);
    }
    return R_SUCCEEDED(rc) ? 0 : -1;
}
