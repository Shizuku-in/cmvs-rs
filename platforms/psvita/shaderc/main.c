/* Offline GXP shader compiler for the Vita player, run once per shader change on a
 * Vita or in Vita3K with a decrypted ur0:data/libshacccg.suprx (Sony's runtime Cg
 * compiler, not redistributable). Compiles every ux0:data/cmvs-shaderc/<name>_v.cg
 * (vertex) and <name>_f.cg (fragment) to <name>_v.gxp / <name>_f.gxp, writes the
 * diagnostics to log.txt and, last, done.txt holding "ok" or "failed". The program
 * binaries are committed under platforms/psvita/shaders; see the README there. */
#include <psp2/io/dirent.h>
#include <psp2/kernel/processmgr.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vitashark.h>

#define DIR "ux0:data/cmvs-shaderc"
#define MAX_SOURCE (256 * 1024)

static FILE *log_file;
static const char *current;

static void on_diagnostic(const char *message, shark_log_level level, int line) {
    static const char *names[] = {"info", "warning", "error"};
    fprintf(log_file, "%s:%d: %s: %s\n", current, line,
            level <= SHARK_LOG_ERROR ? names[level] : "?", message);
}

static int ends_with(const char *name, const char *suffix) {
    size_t n = strlen(name), s = strlen(suffix);
    return n > s && strcmp(name + n - s, suffix) == 0;
}

/* Returns 0 when `name` compiled; the output replaces any earlier binary. */
static int compile(const char *name, shark_type type) {
    char path[256];
    snprintf(path, sizeof(path), DIR "/%s", name);
    FILE *in = fopen(path, "rb");
    if (!in) {
        fprintf(log_file, "%s: cannot open\n", name);
        return -1;
    }
    char *source = malloc(MAX_SOURCE + 1);
    size_t length = source ? fread(source, 1, MAX_SOURCE + 1, in) : 0;
    fclose(in);
    if (!source || length == 0 || length > MAX_SOURCE) {
        fprintf(log_file, "%s: empty, unreadable or larger than %d bytes\n", name, MAX_SOURCE);
        free(source);
        return -1;
    }
    source[length] = 0;
    current = name;
    uint32_t size = (uint32_t)length;
    /* Exact float arithmetic: the scene shader reproduces the software compositor. */
    SceGxmProgram *program = shark_compile_shader_extended(source, &size, type, SHARK_OPT_DEFAULT,
                                                           SHARK_DISABLE, SHARK_DISABLE,
                                                           SHARK_DISABLE);
    free(source);
    int result = -1;
    path[strlen(path) - 3] = 0; /* drop ".cg" */
    strcat(path, ".gxp");
    remove(path);
    if (program) {
        FILE *out = fopen(path, "wb");
        if (out && fwrite(program, 1, size, out) == size && fclose(out) == 0) {
            fprintf(log_file, "%s: %u bytes\n", name, (unsigned)size);
            result = 0;
        } else {
            if (out)
                fclose(out);
            fprintf(log_file, "%s: cannot write the program\n", name);
        }
    } else {
        fprintf(log_file, "%s: compilation failed\n", name);
    }
    shark_clear_output();
    return result;
}

int main(void) {
    remove(DIR "/done.txt");
    log_file = fopen(DIR "/log.txt", "w");
    if (!log_file)
        return sceKernelExitProcess(1);
    int failed = 0;
    /* The simple mode skips SceShaccCgExt's GLSL extensions, which Vita3K cannot load. */
    int init = shark_init_simple(NULL);
    if (init < 0) {
        fprintf(log_file, "cannot load ur0:data/libshacccg.suprx: 0x%08x\n", (unsigned)init);
        failed = 1;
    } else {
        shark_install_log_cb(on_diagnostic);
        shark_set_warnings_level(SHARK_WARN_HIGH);
        SceUID dir = sceIoDopen(DIR);
        SceIoDirent entry;
        int count = 0;
        while (dir >= 0 && sceIoDread(dir, &entry) > 0) {
            if (ends_with(entry.d_name, "_v.cg"))
                failed |= compile(entry.d_name, SHARK_VERTEX_SHADER) != 0, ++count;
            else if (ends_with(entry.d_name, "_f.cg"))
                failed |= compile(entry.d_name, SHARK_FRAGMENT_SHADER) != 0, ++count;
        }
        if (dir >= 0)
            sceIoDclose(dir);
        if (count == 0) {
            fprintf(log_file, "no *_v.cg or *_f.cg in " DIR "\n");
            failed = 1;
        }
        shark_end();
    }
    fclose(log_file);
    FILE *done = fopen(DIR "/done.txt", "w");
    if (done) {
        fputs(failed ? "failed\n" : "ok\n", done);
        fclose(done);
    }
    return sceKernelExitProcess(failed);
}
