#include "clipper_serial_profile.h"

#include <furi.h>
#include <gui/gui.h>
#include <input/input.h>
#include <bt/bt_service/bt.h>
#include <cli/cli.h>
#include <toolbox/pipe.h>
#include <toolbox/cli/cli_registry.h>
#include <toolbox/cli/shell/cli_shell.h>
#include <services/serial_service.h>

#define TAG "Clipper"
#define CLIPPER_PIPE_CAPACITY (2 * 1024)
#define CLIPPER_TX_CHUNK      (BLE_SVC_SERIAL_DATA_LEN_MAX)

typedef struct {
    /* UI */
    FuriMessageQueue* input_queue;
    ViewPort* view_port;
    Gui* gui;

    /* BT / profile */
    Bt* bt;
    FuriHalBleProfileBase* profile;
    BleServiceSerial* serial;
    bool profile_started;

    /* CLI shell bridge */
    CliRegistry* cli_registry;
    PipeSide* own_pipe;   /* we read shell-stdout from here, write BLE-stdin here */
    PipeSide* shell_pipe; /* shell owns this end */
    CliShell* shell;
    FuriThread* bridge_thread;
    FuriEventLoop* bridge_loop;

    /* Stats for UI */
    volatile uint32_t rx_bytes; /* bytes received from BLE host */
    volatile uint32_t tx_bytes; /* bytes sent to BLE host */
} ClipperApp;

/* ------------ UI (always built) ------------ */

static void clipper_render_callback(Canvas* canvas, void* ctx) {
    ClipperApp* app = ctx;
    canvas_clear(canvas);
    canvas_set_font(canvas, FontPrimary);
    canvas_draw_str_aligned(canvas, 64, 12, AlignCenter, AlignTop, "CLIpper");
    canvas_set_font(canvas, FontSecondary);
    canvas_draw_str_aligned(
        canvas, 64, 28, AlignCenter, AlignTop,
        app->profile_started ? "BLE CLI: ready" : "BLE: failed");

    char buf[32];
    snprintf(buf, sizeof(buf), "rx %lu  tx %lu", app->rx_bytes, app->tx_bytes);
    canvas_draw_str_aligned(canvas, 64, 42, AlignCenter, AlignTop, buf);
    canvas_draw_str_aligned(canvas, 64, 55, AlignCenter, AlignTop, "Back to exit");
}

static void clipper_input_callback(InputEvent* event, void* ctx) {
    ClipperApp* app = ctx;
    furi_message_queue_put(app->input_queue, event, FuriWaitForever);
}

static void clipper_redraw_timer_cb(void* ctx) {
    ClipperApp* app = ctx;
    view_port_update(app->view_port);
}

/* ------------ BLE -> CLI shell (RX path) ------------ */

static uint16_t clipper_ble_serial_event(SerialServiceEvent event, void* context) {
    ClipperApp* app = context;
    if(event.event == SerialServiceEventTypeDataReceived) {
        if(app->own_pipe) {
            size_t sent = pipe_send(app->own_pipe, event.data.buffer, event.data.size);
            app->rx_bytes += sent;
        }
    }
    size_t avail = app->own_pipe ? pipe_spaces_available(app->own_pipe) : 0;
    return (uint16_t)(avail > UINT16_MAX ? UINT16_MAX : avail);
}

/* ------------ CLI shell -> BLE (TX path) ------------ */

static void clipper_pipe_data_arrived_cb(PipeSide* pipe, void* context) {
    ClipperApp* app = context;
    uint8_t buf[CLIPPER_TX_CHUNK];
    while(true) {
        size_t avail = pipe_bytes_available(pipe);
        if(avail == 0) break;
        size_t want = avail < sizeof(buf) ? avail : sizeof(buf);
        size_t got = pipe_receive(pipe, buf, want);
        if(got == 0) break;
        if(ble_svc_serial_update_tx(app->serial, buf, got)) {
            app->tx_bytes += got;
        }
    }
}

static int32_t clipper_bridge_thread(void* context) {
    ClipperApp* app = context;
    app->bridge_loop = furi_event_loop_alloc();
    pipe_attach_to_event_loop(app->own_pipe, app->bridge_loop);
    pipe_set_callback_context(app->own_pipe, app);
    pipe_set_data_arrived_callback(
        app->own_pipe, clipper_pipe_data_arrived_cb, FuriEventLoopEventFlagEdge);
    furi_event_loop_run(app->bridge_loop);
    pipe_detach_from_event_loop(app->own_pipe);
    furi_event_loop_free(app->bridge_loop);
    app->bridge_loop = NULL;
    return 0;
}

static void clipper_cli_motd(void* context) {
    UNUSED(context);
    printf("\r\nCLIpper :: BLE CLI shell\r\n");
}

/* ------------ Lifecycle ------------ */

int32_t clipper_app(void* p) {
    UNUSED(p);
    ClipperApp* app = malloc(sizeof(ClipperApp));
    memset(app, 0, sizeof(*app));

    app->input_queue = furi_message_queue_alloc(8, sizeof(InputEvent));
    app->view_port = view_port_alloc();
    view_port_draw_callback_set(app->view_port, clipper_render_callback, app);
    view_port_input_callback_set(app->view_port, clipper_input_callback, app);
    app->gui = furi_record_open(RECORD_GUI);
    gui_add_view_port(app->gui, app->view_port, GuiLayerFullscreen);

    app->bt = furi_record_open(RECORD_BT);
    app->profile = bt_profile_start(app->bt, clipper_serial_profile, NULL);
    app->profile_started = (app->profile != NULL);

    if(app->profile_started) {
        app->serial = clipper_serial_profile_get_service(app->profile);

        app->cli_registry = furi_record_open(RECORD_CLI);
        PipeSideBundle bundle = pipe_alloc(CLIPPER_PIPE_CAPACITY, 1);
        app->own_pipe = bundle.alices_side;
        app->shell_pipe = bundle.bobs_side;
        app->shell = cli_shell_alloc(
            clipper_cli_motd, app, app->shell_pipe, app->cli_registry, NULL);
        cli_shell_start(app->shell);
        app->bridge_thread = furi_thread_alloc_ex(
            "ClipperBridge", 2048, clipper_bridge_thread, app);
        furi_thread_start(app->bridge_thread);
        ble_svc_serial_set_callbacks(
            app->serial, CLIPPER_PIPE_CAPACITY, clipper_ble_serial_event, app);
        ble_svc_serial_set_rpc_active(app->serial, false);
        FURI_LOG_I(TAG, "BLE CLI bridge ready");
    } else {
        FURI_LOG_E(TAG, "bt_profile_start returned NULL");
    }

    FuriTimer* refresh = furi_timer_alloc(clipper_redraw_timer_cb, FuriTimerTypePeriodic, app);
    furi_timer_start(refresh, 500);

    InputEvent event;
    while(true) {
        if(furi_message_queue_get(app->input_queue, &event, FuriWaitForever) == FuriStatusOk) {
            if(event.type == InputTypePress && event.key == InputKeyBack) break;
        }
    }

    furi_timer_stop(refresh);
    furi_timer_free(refresh);

    /* Shutdown order — must be careful about who owns what when:
     *   1. Stop the BLE -> shell path (no more incoming data).
     *   2. Tell the bridge event loop to stop and join its thread. The
     *      bridge thread detaches own_pipe from its loop on the way out, so
     *      we MUST NOT free own_pipe before the thread joins.
     *   3. Now safe to free our pipe end; this makes the shell's pipe go
     *      broken, so its thread will exit on the next read.
     *   4. Join + free the shell.
     *   5. Restore default BT profile (frees our BleServiceSerial). */
    if(app->profile_started) {
        ble_svc_serial_set_callbacks(app->serial, 0, NULL, NULL);

        if(app->bridge_loop) furi_event_loop_stop(app->bridge_loop);
        furi_thread_join(app->bridge_thread);
        furi_thread_free(app->bridge_thread);

        if(app->own_pipe) {
            pipe_free(app->own_pipe);
            app->own_pipe = NULL;
        }

        cli_shell_join(app->shell);
        cli_shell_free(app->shell);
        pipe_free(app->shell_pipe);

        furi_record_close(RECORD_CLI);
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
