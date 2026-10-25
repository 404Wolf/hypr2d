// VM-only Wayland virtual-pointer client; Python owns the scripted interactions.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>
#include "pointer-protocol.h"
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(registry, name, &zwlr_virtual_pointer_manager_v1_interface, version < 2 ? version : 2);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {(void)data;(void)registry;(void)name;}
int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 1;
    struct wl_registry *registry = wl_display_get_registry(display);
    const struct wl_registry_listener listener = {global, removed};
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !manager) return 2;
    struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, NULL);
    char command[128];
    puts("ready");fflush(stdout);
    while (fgets(command, sizeof(command), stdin)) {
        double dx,dy;
        if (sscanf(command,"move %lf %lf",&dx,&dy)==2)
            zwlr_virtual_pointer_v1_motion(pointer, 0, wl_fixed_from_double(dx), wl_fixed_from_double(dy));
        else if (!strncmp(command,"down",4)) zwlr_virtual_pointer_v1_button(pointer,0,272,WL_POINTER_BUTTON_STATE_PRESSED);
        else if (!strncmp(command,"up",2)) zwlr_virtual_pointer_v1_button(pointer,0,272,WL_POINTER_BUTTON_STATE_RELEASED);
        else if (!strncmp(command,"quit",4)) break;
        else return 3;
        zwlr_virtual_pointer_v1_frame(pointer);
        if (wl_display_roundtrip(display)<0) return 4;
        puts("ok");fflush(stdout);
    }
    zwlr_virtual_pointer_v1_destroy(pointer);
    zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_registry_destroy(registry);
    wl_display_flush(display);
    wl_display_disconnect(display);
    return 0;
}
