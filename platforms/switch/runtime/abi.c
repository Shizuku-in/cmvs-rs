#include <sys/stat.h>
#include <dirent.h>
#include <stddef.h>
/* Fail the build if the pinned newlib layout changes. */
_Static_assert(sizeof(dev_t) == 2, "dev_t");
_Static_assert(sizeof(ino_t) == 2, "ino_t");
_Static_assert(sizeof(blksize_t) == 8, "blksize_t");
_Static_assert(sizeof(blkcnt_t) == 8, "blkcnt_t");
_Static_assert(offsetof(struct stat, st_mode) == 4, "stat mode");
_Static_assert(offsetof(struct dirent, d_name) == 3, "dirent name");
