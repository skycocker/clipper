#include "clipper_test_profile.h"

#include <furi.h>
#include <gui/gui.h>
#include <input/input.h>
#include <bt/bt_service/bt.h>

typedef struct {
    FuriMessageQueue* input_queue;
    ViewPort* view_port;
    Gui* gui;
    Bt* bt;
    FuriHalBleProfileBase* profile;
    bool profile_started;
} ClipperApp;

static void clipper_render_callback(Canvas* canvas, void* ctx) {
    ClipperApp* app = ctx;
    canvas_clear(canvas);
    canvas_set_font(canvas, FontPrimary);
    canvas_draw_str_aligned(canvas, 64, 14, AlignCenter, AlignTop, "Clipper");
    canvas_set_font(canvas, FontSecondary);
    canvas_draw_str_aligned(
        canvas, 64, 32, AlignCenter, AlignTop,
        app->profile_started ? "BLE: Active" : "BLE: failed");
    canvas_draw_str_aligned(canvas, 64, 50, AlignCenter, AlignTop, "Back to exit");
}

static void clipper_input_callback(InputEvent* event, void* ctx) {
    ClipperApp* app = ctx;
    furi_message_queue_put(app->input_queue, event, FuriWaitForever);
}

int32_t clipper_app(void* p) {
    UNUSED(p);
    ClipperApp* app = malloc(sizeof(ClipperApp));
    app->input_queue = furi_message_queue_alloc(8, sizeof(InputEvent));
    app->view_port = view_port_alloc();
    view_port_draw_callback_set(app->view_port, clipper_render_callback, app);
    view_port_input_callback_set(app->view_port, clipper_input_callback, app);
    app->gui = furi_record_open(RECORD_GUI);
    gui_add_view_port(app->gui, app->view_port, GuiLayerFullscreen);

    app->bt = furi_record_open(RECORD_BT);
    app->profile = bt_profile_start(app->bt, clipper_test_profile, NULL);
    app->profile_started = (app->profile != NULL);
    if(app->profile_started) {
        FURI_LOG_I("Clipper", "Test profile started, advertising as 'Clipper'");
    } else {
        FURI_LOG_E("Clipper", "bt_profile_start returned NULL");
    }

    InputEvent event;
    bool running = true;
    while(running) {
        if(furi_message_queue_get(app->input_queue, &event, FuriWaitForever) == FuriStatusOk) {
            if(event.type == InputTypePress && event.key == InputKeyBack) {
                running = false;
            }
        }
    }

    /* Must restore default profile BEFORE closing the bt record (otherwise the
     * bt service still holds a pointer to our template, which lives in this
     * .fap's memory and is about to unload). */
    if(app->profile_started) {
        bt_profile_restore_default(app->bt);
    }
    furi_record_close(RECORD_BT);

    view_port_enabled_set(app->view_port, false);
    gui_remove_view_port(app->gui, app->view_port);
    view_port_free(app->view_port);
    furi_message_queue_free(app->input_queue);
    furi_record_close(RECORD_GUI);
    free(app);
    return 0;
}
