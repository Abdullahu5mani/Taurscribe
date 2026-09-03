#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <linux/uinput.h>
#include <sys/ioctl.h>

int main() {
    int fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK);
    if (fd < 0) {
        perror("[UINPUT] open /dev/uinput");
        return 1;
    }
    printf("[UINPUT] Successfully opened /dev/uinput (fd: %d)\n", fd);

    if (ioctl(fd, UI_SET_EVBIT, EV_KEY) < 0) {
        perror("[UINPUT] ioctl UI_SET_EVBIT EV_KEY");
        close(fd);
        return 1;
    }
    if (ioctl(fd, UI_SET_EVBIT, EV_SYN) < 0) {
        perror("[UINPUT] ioctl UI_SET_EVBIT EV_SYN");
        close(fd);
        return 1;
    }
    if (ioctl(fd, UI_SET_KEYBIT, KEY_LEFTCTRL) < 0) {
        perror("[UINPUT] ioctl UI_SET_KEYBIT KEY_LEFTCTRL");
        close(fd);
        return 1;
    }
    if (ioctl(fd, UI_SET_KEYBIT, KEY_V) < 0) {
        perror("[UINPUT] ioctl UI_SET_KEYBIT KEY_V");
        close(fd);
        return 1;
    }

    struct uinput_setup usetup;
    memset(&usetup, 0, sizeof(usetup));
    usetup.id.bustype = BUS_USB;
    usetup.id.vendor = 0x1234;
    usetup.id.product = 0x5678;
    strncpy(usetup.name, "Taurscribe Virtual Keyboard", UINPUT_MAX_NAME_SIZE);

    if (ioctl(fd, UI_DEV_SETUP, &usetup) < 0) {
        perror("[UINPUT] ioctl UI_DEV_SETUP failed");
        close(fd);
        return 1;
    }

    if (ioctl(fd, UI_DEV_CREATE) < 0) {
        perror("[UINPUT] ioctl UI_DEV_CREATE failed");
        close(fd);
        return 1;
    }
    printf("[UINPUT] Successfully created synthetic virtual keyboard device via UI_DEV_CREATE!\n");

    // Emit Ctrl+V
    struct input_event ev;
    memset(&ev, 0, sizeof(ev));
    ev.type = EV_KEY;
    ev.code = KEY_LEFTCTRL;
    ev.value = 1;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_SYN;
    ev.code = SYN_REPORT;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_KEY;
    ev.code = KEY_V;
    ev.value = 1;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_SYN;
    ev.code = SYN_REPORT;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_KEY;
    ev.code = KEY_V;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_SYN;
    ev.code = SYN_REPORT;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_KEY;
    ev.code = KEY_LEFTCTRL;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    ev.type = EV_SYN;
    ev.code = SYN_REPORT;
    ev.value = 0;
    write(fd, &ev, sizeof(ev));

    printf("[UINPUT] Successfully emitted synthetic Ctrl+V key events!\n");

    if (ioctl(fd, UI_DEV_DESTROY) < 0) {
        perror("[UINPUT] ioctl UI_DEV_DESTROY failed");
    } else {
        printf("[UINPUT] Virtual keyboard destroyed cleanly.\n");
    }

    close(fd);
    return 0;
}
