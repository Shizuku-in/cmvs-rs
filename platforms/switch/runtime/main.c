#include <switch.h>
#include <stdio.h>

extern int cmvs_switch_probe(void);
int main(void) {
    consoleInit(NULL);
    PadState pad;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    printf("CMVS Switch bootstrap\n\nToolchain and libnx smoke test.\nGame runtime not connected.\nPress + to exit.\n");
    printf("Rust/core probe status: %d\n", cmvs_switch_probe());
    while (appletMainLoop()) {
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & HidNpadButton_Plus) break;
        consoleUpdate(NULL);
    }
    consoleExit(NULL);
    return 0;
}
