#include <switch.h>
#include <errno.h>
#include <sys/types.h>
#include <unistd.h>

/* POSIX calls Rust std makes on Horizon that libnx's newlib leaves undefined. */
_Static_assert(_SC_PAGESIZE == 8, "libc crate horizon _SC_PAGESIZE");

/* std::thread spawning rounds stack sizes to the page size. */
long sysconf(int name) {
    if (name == _SC_PAGESIZE)
        return 0x1000;
    errno = EINVAL;
    return -1;
}

/* HashMap seeds; libnx's csrng never fails and returns every requested byte. */
ssize_t getrandom(void *buffer, size_t length, unsigned flags) {
    (void)flags;
    randomGet(buffer, length);
    return (ssize_t)length;
}
