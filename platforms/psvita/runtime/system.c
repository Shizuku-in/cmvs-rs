#include <psp2/kernel/threadmgr.h>
#include <psp2/power.h>

/* The heap takes most of user memory (the SFO's ATTRIBUTE2=12 extends it); the main
   thread runs the VM and the software compositor. */
unsigned int _newlib_heap_size_user = 256 * 1024 * 1024;
unsigned int sceUserMainThreadStackSize = 4 * 1024 * 1024;

/* The highest application clocks: CPU 444 MHz, bus 222, GPU 222, crossbar 166. The
   system restores its defaults when the application exits. */
int cmvs_vita_set_max_clocks(void) {
    int result = scePowerSetArmClockFrequency(444);
    scePowerSetBusClockFrequency(222);
    scePowerSetGpuClockFrequency(222);
    scePowerSetGpuXbarClockFrequency(166);
    return result < 0 ? -1 : 0;
}

/* Moves the calling thread to user core 0, 1 or 2. */
int cmvs_vita_prefer_core(int core) {
    if (core < 0 || core > 2)
        return -1;
    return sceKernelChangeThreadCpuAffinityMask(0, SCE_KERNEL_CPU_MASK_USER_0 << core) < 0 ? -1 : 0;
}
