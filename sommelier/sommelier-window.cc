// Copyright 2021 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "sommelier-window.h"  // NOLINT(build/include_directory)

#include <assert.h>

#include "sommelier.h"            // NOLINT(build/include_directory)
#include "sommelier-tracing.h"    // NOLINT(build/include_directory)
#include "sommelier-transform.h"  // NOLINT(build/include_directory)

#include "aura-shell-client-protocol.h"  // NOLINT(build/include_directory)
#include "xdg-shell-client-protocol.h"   // NOLINT(build/include_directory)

#define APPLICATION_ID_FORMAT_PREFIX "org.chromium.guest_os.%s"
#define XID_APPLICATION_ID_FORMAT APPLICATION_ID_FORMAT_PREFIX ".xid.%d"
#define WM_CLIENT_LEADER_APPLICATION_ID_FORMAT \
  APPLICATION_ID_FORMAT_PREFIX ".wmclientleader.%d"
#define WM_CLASS_APPLICATION_ID_FORMAT \
  APPLICATION_ID_FORMAT_PREFIX ".wmclass.%s"
#define X11_PROPERTY_APPLICATION_ID_FORMAT \
  APPLICATION_ID_FORMAT_PREFIX ".xprop.%s"
sl_window::sl_window(struct sl_context* ctx,
                     xcb_window_t id,
                     int x,
                     int y,
                     int width,
                     int height,
                     int border_width)
    : ctx(ctx),
      id(id),
      x(x),
      y(y),
      width(width),
      height(height),
      border_width(border_width) {
  wl_list_insert(&ctx->unpaired_windows, &link);
  pixman_region32_init(&shape_rectangles);
}

sl_window::~sl_window() {
  if (this == ctx->host_focus_window) {
    ctx->host_focus_window = nullptr;
    ctx->needs_set_input_focus = 1;
  }

  free(name);
  free(clazz);
  free(startup_id);
  wl_list_remove(&link);
  pixman_region32_fini(&shape_rectangles);
}

void sl_configure_window(struct sl_window* window) {
  TRACE_EVENT("surface", "sl_configure_window", "id", window->id);
  assert(!window->pending_config.serial);

  if (window->next_config.mask) {
    int values[5];
    int x = window->x;
    int y = window->y;
    int i = 0;

    xcb_configure_window(window->ctx->connection, window->frame_id,
                         window->next_config.mask, window->next_config.values);

    if (window->next_config.mask & XCB_CONFIG_WINDOW_X)
      x = window->next_config.values[i++];
    if (window->next_config.mask & XCB_CONFIG_WINDOW_Y)
      y = window->next_config.values[i++];
    if (window->next_config.mask & XCB_CONFIG_WINDOW_WIDTH)
      window->width = window->next_config.values[i++];
    if (window->next_config.mask & XCB_CONFIG_WINDOW_HEIGHT)
      window->height = window->next_config.values[i++];
    if (window->next_config.mask & XCB_CONFIG_WINDOW_BORDER_WIDTH)
      window->border_width = window->next_config.values[i++];

    // Set x/y to origin in case window gravity is not northwest as expected.
    assert(window->managed);
    values[0] = 0;
    values[1] = 0;
    values[2] = window->width;
    values[3] = window->height;
    values[4] = window->border_width;
    xcb_configure_window(
        window->ctx->connection, window->id,
        XCB_CONFIG_WINDOW_X | XCB_CONFIG_WINDOW_Y | XCB_CONFIG_WINDOW_WIDTH |
            XCB_CONFIG_WINDOW_HEIGHT | XCB_CONFIG_WINDOW_BORDER_WIDTH,
        values);

    if (x != window->x || y != window->y) {
      window->x = x;
      window->y = y;
      sl_send_configure_notify(window);
    }
  }

  if (window->managed) {
    xcb_change_property(window->ctx->connection, XCB_PROP_MODE_REPLACE,
                        window->id, window->ctx->atoms[ATOM_NET_WM_STATE].value,
                        XCB_ATOM_ATOM, 32, window->next_config.states_length,
                        window->next_config.states);
  }

  window->pending_config = window->next_config;
  window->next_config.serial = 0;
  window->next_config.mask = 0;
  window->next_config.states_length = 0;
}

void sl_send_configure_notify(struct sl_window* window) {
  xcb_configure_notify_event_t event = {};
  event.response_type = XCB_CONFIGURE_NOTIFY;
  event.pad0 = 0;
  event.event = window->id;
  event.window = window->id;
  event.above_sibling = XCB_WINDOW_NONE;
  event.x = static_cast<int16_t>(window->x);
  event.y = static_cast<int16_t>(window->y);
  event.width = static_cast<uint16_t>(window->width);
  event.height = static_cast<uint16_t>(window->height);
  event.border_width = static_cast<uint16_t>(window->border_width);
  event.override_redirect = 0;
  event.pad1 = 0;

  xcb_send_event(window->ctx->connection, 0, window->id,
                 XCB_EVENT_MASK_STRUCTURE_NOTIFY,
                 reinterpret_cast<char*>(&event));
}

int sl_process_pending_configure_acks(struct sl_window* window,
                                      struct sl_host_surface* host_surface) {
  if (!window->pending_config.serial)
    return 0;

#ifdef COMMIT_LOOP_FIX
  // Do not commit/ack if there is nothing to change.
  //
  // TODO(b/181077580): we should never do this, but avoiding it requires a
  // more systemic fix
  if (!window->pending_config.mask && window->pending_config.states_length == 0)
    return 0;
#endif

  if (window->managed && host_surface) {
    uint32_t width = window->width + window->border_width * 2;
    uint32_t height = window->height + window->border_width * 2;
    // Early out if we expect contents to match window size at some point in
    // the future.
    if (width != host_surface->contents_width ||
        height != host_surface->contents_height) {
      return 0;
    }
  }

  if (window->xdg_surface) {
    xdg_surface_ack_configure(window->xdg_surface,
                              window->pending_config.serial);
  }
  window->pending_config.serial = 0;

  if (window->next_config.serial)
    sl_configure_window(window);

  return 1;
}

void sl_commit(struct sl_window* window, struct sl_host_surface* host_surface) {
  if (sl_process_pending_configure_acks(window, host_surface)) {
    if (host_surface)
      wl_surface_commit(host_surface->proxy);
  }
}

static void sl_internal_xdg_popup_configure(void* data,
                                            struct xdg_popup* xdg_popup,
                                            int32_t x,
                                            int32_t y,
                                            int32_t width,
                                            int32_t height) {}

static void sl_internal_xdg_popup_done(void* data,
                                       struct xdg_popup* xdg_popup) {}

static const struct xdg_popup_listener sl_internal_xdg_popup_listener = {
    sl_internal_xdg_popup_configure, sl_internal_xdg_popup_done};

static void sl_internal_xdg_surface_configure(void* data,
                                              struct xdg_surface* xdg_surface,
                                              uint32_t serial) {
  TRACE_EVENT("surface", "sl_internal_xdg_surface_configure");
  struct sl_window* window =
      static_cast<sl_window*>(xdg_surface_get_user_data(xdg_surface));

  window->next_config.serial = serial;
  if (!window->pending_config.serial) {
    struct wl_resource* host_resource;
    struct sl_host_surface* host_surface = NULL;

    host_resource =
        wl_client_get_object(window->ctx->client, window->host_surface_id);
    if (host_resource)
      host_surface = static_cast<sl_host_surface*>(
          wl_resource_get_user_data(host_resource));

    sl_configure_window(window);
    sl_commit(window, host_surface);
  }
}

static const struct xdg_surface_listener sl_internal_xdg_surface_listener = {
    sl_internal_xdg_surface_configure};

static void sl_internal_xdg_toplevel_configure(
    void* data,
    struct xdg_toplevel* xdg_toplevel,
    int32_t width,
    int32_t height,
    struct wl_array* states) {
  TRACE_EVENT("other", "sl_internal_xdg_toplevel_configure");
  struct sl_window* window =
      static_cast<sl_window*>(xdg_toplevel_get_user_data(xdg_toplevel));
  int activated = 0;
  uint32_t* state;
  int i = 0;

  if (!window->managed)
    return;

  if (width && height) {
    int32_t width_in_pixels = width;
    int32_t height_in_pixels = height;
    int i = 0;

    // We are receiving a request to resize a window (in logical dimensions)
    // If the request is equal to the cached values we used to make adjustments
    // do not recalculate the values
    // However, if the request is not equal to the cached values, try
    // and keep the buffer same size as what was previously set
    // by the application.
    struct sl_host_surface* paired_surface = window->paired_surface;

    if (paired_surface && paired_surface->has_own_scale) {
      if (width != paired_surface->cached_logical_width ||
          height != paired_surface->cached_logical_height) {
        sl_transform_try_window_scale(window->ctx, paired_surface,
                                      window->width, window->height);
      }
    }

    sl_transform_host_to_guest(window->ctx, window->paired_surface,
                               &width_in_pixels, &height_in_pixels);
    window->next_config.mask = XCB_CONFIG_WINDOW_WIDTH |
                               XCB_CONFIG_WINDOW_HEIGHT |
                               XCB_CONFIG_WINDOW_BORDER_WIDTH;
    if (!(window->size_flags & (US_POSITION | P_POSITION))) {
      window->next_config.mask |= XCB_CONFIG_WINDOW_X | XCB_CONFIG_WINDOW_Y;
      if (window->paired_surface && window->paired_surface->output) {
        window->next_config.values[i++] =
            window->paired_surface->output->virt_x +
            (window->paired_surface->output->width - width_in_pixels) / 2;
        window->next_config.values[i++] =
            window->paired_surface->output->virt_y +
            (window->paired_surface->output->height - height_in_pixels) / 2;
      } else {
        window->next_config.values[i++] =
            window->ctx->screen->width_in_pixels / 2 - width_in_pixels / 2;
        window->next_config.values[i++] =
            window->ctx->screen->height_in_pixels / 2 - height_in_pixels / 2;
      }
    }
    window->next_config.values[i++] = width_in_pixels;
    window->next_config.values[i++] = height_in_pixels;
    window->next_config.values[i++] = 0;
  }

  window->allow_resize = 1;
  window->compositor_fullscreen = 0;
  sl_array_for_each(state, states) {
    if (*state == XDG_TOPLEVEL_STATE_FULLSCREEN) {
      window->allow_resize = 0;
      window->next_config.states[i++] =
          window->ctx->atoms[ATOM_NET_WM_STATE_FULLSCREEN].value;
      window->compositor_fullscreen = 1;
    }
    if (*state == XDG_TOPLEVEL_STATE_MAXIMIZED) {
      window->allow_resize = 0;
      window->next_config.states[i++] =
          window->ctx->atoms[ATOM_NET_WM_STATE_MAXIMIZED_VERT].value;
      window->next_config.states[i++] =
          window->ctx->atoms[ATOM_NET_WM_STATE_MAXIMIZED_HORZ].value;
    }
    if (*state == XDG_TOPLEVEL_STATE_ACTIVATED) {
      activated = 1;
      window->next_config.states[i++] =
          window->ctx->atoms[ATOM_NET_WM_STATE_FOCUSED].value;
    }
    if (*state == XDG_TOPLEVEL_STATE_RESIZING)
      window->allow_resize = 0;
  }

  if (activated != window->activated) {
    if (activated != (window->ctx->host_focus_window == window)) {
      window->ctx->host_focus_window = activated ? window : NULL;
      window->ctx->needs_set_input_focus = 1;
    }
    window->activated = activated;
  }

  window->next_config.states_length = i;
}

static void sl_internal_xdg_toplevel_close(void* data,
                                           struct xdg_toplevel* xdg_toplevel) {
  TRACE_EVENT("other", "sl_internal_xdg_toplevel_close");
  struct sl_window* window =
      static_cast<sl_window*>(xdg_toplevel_get_user_data(xdg_toplevel));
  xcb_client_message_event_t event = {};
  event.response_type = XCB_CLIENT_MESSAGE;
  event.format = 32;
  event.window = window->id;
  event.type = window->ctx->atoms[ATOM_WM_PROTOCOLS].value;
  event.data.data32[0] = window->ctx->atoms[ATOM_WM_DELETE_WINDOW].value;
  event.data.data32[1] = XCB_CURRENT_TIME;

  xcb_send_event(window->ctx->connection, 0, window->id,
                 XCB_EVENT_MASK_NO_EVENT, (const char*)&event);
}

static const struct xdg_toplevel_listener sl_internal_xdg_toplevel_listener = {
    sl_internal_xdg_toplevel_configure, sl_internal_xdg_toplevel_close};
void sl_update_application_id(struct sl_context* ctx,
                              struct sl_window* window) {
  TRACE_EVENT("other", "sl_update_application_id");
  if (!window->aura_surface)
    return;
  if (ctx->application_id) {
    zaura_surface_set_application_id(window->aura_surface, ctx->application_id);
    return;
  }
  // Don't set application id for X11 override redirect. This prevents
  // aura shell from thinking that these are regular application windows
  // that should appear in application lists.
  if (!ctx->xwayland || window->managed) {
    char* application_id_str;
    if (!window->app_id_property.empty()) {
      application_id_str =
          sl_xasprintf(X11_PROPERTY_APPLICATION_ID_FORMAT, ctx->vm_id,
                       window->app_id_property.c_str());
    } else if (window->clazz) {
      application_id_str = sl_xasprintf(WM_CLASS_APPLICATION_ID_FORMAT,
                                        ctx->vm_id, window->clazz);
    } else if (window->client_leader != XCB_WINDOW_NONE) {
      application_id_str = sl_xasprintf(WM_CLIENT_LEADER_APPLICATION_ID_FORMAT,
                                        ctx->vm_id, window->client_leader);
    } else {
      application_id_str =
          sl_xasprintf(XID_APPLICATION_ID_FORMAT, ctx->vm_id, window->id);
    }

    zaura_surface_set_application_id(window->aura_surface, application_id_str);
    free(application_id_str);
  }
}

void sl_window_update(struct sl_window* window) {
  TRACE_EVENT("surface", "sl_window_update", "id", window->id);
  struct wl_resource* host_resource = NULL;
  struct sl_host_surface* host_surface;
  struct sl_context* ctx = window->ctx;
  struct sl_window* parent = NULL;

  if (window->host_surface_id) {
    host_resource = wl_client_get_object(ctx->client, window->host_surface_id);
    if (host_resource && window->unpaired) {
      wl_list_remove(&window->link);
      wl_list_insert(&ctx->windows, &window->link);
      window->unpaired = 0;
    }
  } else if (!window->unpaired) {
    wl_list_remove(&window->link);
    wl_list_insert(&ctx->unpaired_windows, &window->link);
    window->unpaired = 1;
    window->paired_surface = NULL;
  }

  if (!host_resource) {
    if (window->aura_surface) {
      zaura_surface_destroy(window->aura_surface);
      window->aura_surface = NULL;
    }
    if (window->xdg_toplevel) {
      xdg_toplevel_destroy(window->xdg_toplevel);
      window->xdg_toplevel = NULL;
    }
    if (window->xdg_popup) {
      xdg_popup_destroy(window->xdg_popup);
      window->xdg_popup = NULL;
    }
    if (window->xdg_surface) {
      xdg_surface_destroy(window->xdg_surface);
      window->xdg_surface = NULL;
    }
    window->realized = 0;
    return;
  }

  host_surface =
      static_cast<sl_host_surface*>(wl_resource_get_user_data(host_resource));
  assert(host_surface);
  assert(!host_surface->has_role);

  if (!window->unpaired) {
    window->paired_surface = host_surface;
    sl_transform_try_window_scale(ctx, host_surface, window->width,
                                  window->height);
  }

  assert(ctx->xdg_shell);
  assert(ctx->xdg_shell->internal);

  if (window->managed) {
    if (window->transient_for != XCB_WINDOW_NONE) {
      struct sl_window* sibling;

      wl_list_for_each(sibling, &ctx->windows, link) {
        if (sibling->id == window->transient_for) {
          if (sibling->xdg_toplevel)
            parent = sibling;
          break;
        }
      }
    }
  }

  // If we have a transient parent, but could not find it in the list of
  // realized windows, then pick the window that had the last event for the
  // parent.  We update this again when we gain focus, so if we picked the wrong
  // one it can get corrected at that point (but it's also possible the parent
  // will never be realized, which is why selecting one here is important).
  if (!window->managed ||
      (!parent && window->transient_for != XCB_WINDOW_NONE)) {
    struct sl_window* sibling;
    uint32_t parent_last_event_serial = 0;

    wl_list_for_each(sibling, &ctx->windows, link) {
      struct wl_resource* sibling_host_resource;
      struct sl_host_surface* sibling_host_surface;

      if (!sibling->realized)
        continue;

      sibling_host_resource =
          wl_client_get_object(ctx->client, sibling->host_surface_id);
      if (!sibling_host_resource)
        continue;

      // Any parent will do but prefer last event window.
      sibling_host_surface = static_cast<sl_host_surface*>(
          wl_resource_get_user_data(sibling_host_resource));
      if (parent_last_event_serial > sibling_host_surface->last_event_serial)
        continue;

      // Do not use ourselves as the parent.
      if (sibling->host_surface_id == window->host_surface_id)
        continue;

      parent = sibling;
      parent_last_event_serial = sibling_host_surface->last_event_serial;
    }
  }

  if (!window->depth) {
    xcb_get_geometry_reply_t* geometry_reply = xcb_get_geometry_reply(
        ctx->connection, xcb_get_geometry(ctx->connection, window->id), NULL);
    if (geometry_reply) {
      window->depth = geometry_reply->depth;
      free(geometry_reply);
    }
  }

  if (!window->xdg_surface) {
    window->xdg_surface = xdg_wm_base_get_xdg_surface(ctx->xdg_shell->internal,
                                                      host_surface->proxy);
    xdg_surface_add_listener(window->xdg_surface,
                             &sl_internal_xdg_surface_listener, window);
  }

  if (ctx->aura_shell) {
    uint32_t frame_color;

    if (!window->aura_surface) {
      window->aura_surface = zaura_shell_get_aura_surface(
          ctx->aura_shell->internal, host_surface->proxy);
    }

    zaura_surface_set_frame(window->aura_surface,
                            window->decorated
                                ? ZAURA_SURFACE_FRAME_TYPE_NORMAL
                                : window->depth == 32
                                      ? ZAURA_SURFACE_FRAME_TYPE_NONE
                                      : ZAURA_SURFACE_FRAME_TYPE_SHADOW);

    frame_color = window->dark_frame ? ctx->dark_frame_color : ctx->frame_color;
    zaura_surface_set_frame_colors(window->aura_surface, frame_color,
                                   frame_color);
    zaura_surface_set_startup_id(window->aura_surface, window->startup_id);
    sl_update_application_id(ctx, window);

    if (ctx->aura_shell->version >=
        ZAURA_SURFACE_SET_FULLSCREEN_MODE_SINCE_VERSION) {
      zaura_surface_set_fullscreen_mode(window->aura_surface,
                                        ctx->fullscreen_mode);
    }
  }

  // Always use top-level surface for X11 windows as we can't control when the
  // window is closed.
  if (ctx->xwayland || !parent) {
    if (!window->xdg_toplevel) {
      window->xdg_toplevel = xdg_surface_get_toplevel(window->xdg_surface);
      xdg_toplevel_add_listener(window->xdg_toplevel,
                                &sl_internal_xdg_toplevel_listener, window);
    }
    if (parent)
      xdg_toplevel_set_parent(window->xdg_toplevel, parent->xdg_toplevel);
    if (window->name)
      xdg_toplevel_set_title(window->xdg_toplevel, window->name);
    if (window->size_flags & P_MIN_SIZE) {
      int32_t minw = window->min_width;
      int32_t minh = window->min_height;

      sl_transform_guest_to_host(window->ctx, window->paired_surface, &minw,
                                 &minh);
      xdg_toplevel_set_min_size(window->xdg_toplevel, minw, minh);
    }
    if (window->size_flags & P_MAX_SIZE) {
      int32_t maxw = window->max_width;
      int32_t maxh = window->max_height;

      sl_transform_guest_to_host(window->ctx, window->paired_surface, &maxw,
                                 &maxh);
      xdg_toplevel_set_max_size(window->xdg_toplevel, maxw, maxh);
    }
    if (window->maximized) {
      xdg_toplevel_set_maximized(window->xdg_toplevel);
    }
    if (window->fullscreen) {
      xdg_toplevel_set_fullscreen(window->xdg_toplevel, NULL);
    }
  } else if (!window->xdg_popup) {
    struct xdg_positioner* positioner;
    int32_t diffx = window->x - parent->x;
    int32_t diffy = window->y - parent->y;

    positioner = xdg_wm_base_create_positioner(ctx->xdg_shell->internal);
    assert(positioner);

    sl_transform_guest_to_host(window->ctx, window->paired_surface, &diffx,
                               &diffy);
    xdg_positioner_set_anchor(positioner, XDG_POSITIONER_ANCHOR_TOP_LEFT);
    xdg_positioner_set_gravity(positioner, XDG_POSITIONER_GRAVITY_BOTTOM_RIGHT);
    xdg_positioner_set_anchor_rect(positioner, diffx, diffy, 1, 1);

    window->xdg_popup = xdg_surface_get_popup(window->xdg_surface,
                                              parent->xdg_surface, positioner);
    xdg_popup_add_listener(window->xdg_popup, &sl_internal_xdg_popup_listener,
                           window);

    xdg_positioner_destroy(positioner);
  }

  if ((window->size_flags & (US_POSITION | P_POSITION)) && parent &&
      ctx->aura_shell) {
    int32_t diffx = window->x - parent->x;
    int32_t diffy = window->y - parent->y;

    sl_transform_guest_to_host(window->ctx, window->paired_surface, &diffx,
                               &diffy);
    zaura_surface_set_parent(window->aura_surface, parent->aura_surface, diffx,
                             diffy);
  }

#ifdef COMMIT_LOOP_FIX
  sl_commit(window, host_surface);
#else
  wl_surface_commit(host_surface->proxy);
#endif

  if (host_surface->contents_width && host_surface->contents_height)
    window->realized = 1;
}
