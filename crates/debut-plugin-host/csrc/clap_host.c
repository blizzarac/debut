/* A CLAP audio-effect host (AUD-09): one stereo main input and output,
 * parameter changes sent as events, offered nothing optional by the host.
 *
 * Runs inside the plugin-host helper process, like the OpenFX host. Audio
 * arrives interleaved; the plugin sees de-interleaved float buffers. A mono
 * plugin gets the left/right average and its output goes to both sides. */

#include <dlfcn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "clap/entry.h"
#include "clap/events.h"
#include "clap/ext/audio-ports.h"
#include "clap/ext/params.h"
#include "clap/factory/plugin-factory.h"
#include "clap/host.h"
#include "clap/plugin.h"
#include "clap/process.h"

typedef void (*debut_param_cb)(void *ctx, const char *name, const char *label, const char *kind,
                               int dims, double def, double min, double max);

#define MAX_EVENTS 256

typedef struct library {
  char *path;
  void *dl;
  const clap_plugin_entry_t *entry;
  const clap_plugin_factory_t *factory;
  struct library *next;
} library;

typedef struct {
  clap_id id;
  char name[CLAP_NAME_SIZE];
  double sent;
} param_slot;

typedef struct {
  const clap_plugin_t *plugin;
  const clap_plugin_params_t *params;
  int in_ch, out_ch, max_frames;
  int active, processing;
  float *in[2], *out[2];
  param_slot *slots;
  int nslots;
  clap_event_param_value_t pending[MAX_EVENTS];
  int npending;
  int64_t steady;
} instance;

static library *libraries;

static void set_err(char *err, int cap, const char *fmt, ...) {
  if (!err || cap <= 0) return;
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(err, (size_t)cap, fmt, ap);
  va_end(ap);
}

static const void *host_get_extension(const clap_host_t *h, const char *id) {
  (void)h, (void)id;
  return NULL;
}
static void host_request(const clap_host_t *h) { (void)h; }

static const clap_host_t host = {
    CLAP_VERSION_INIT,   NULL,         "debut",      "debut", "https://github.com/blizzarac/debut",
    "0.1",               host_get_extension, host_request, host_request, host_request,
};

static library *open_library(const char *path, char *err, int cap) {
  for (library *l = libraries; l; l = l->next)
    if (!strcmp(l->path, path)) return l;
  void *dl = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  if (!dl) {
    set_err(err, cap, "cannot load %s: %s", path, dlerror());
    return NULL;
  }
  const clap_plugin_entry_t *entry = dlsym(dl, "clap_entry");
  if (!entry) {
    set_err(err, cap, "%s is not a CLAP plugin", path);
    dlclose(dl);
    return NULL;
  }
  if (!clap_version_is_compatible(entry->clap_version)) {
    set_err(err, cap, "%s uses CLAP %u.%u, which this host does not speak", path,
            entry->clap_version.major, entry->clap_version.minor);
    dlclose(dl);
    return NULL;
  }
  if (!entry->init(path)) {
    set_err(err, cap, "%s failed to initialize", path);
    dlclose(dl);
    return NULL;
  }
  const clap_plugin_factory_t *factory = entry->get_factory(CLAP_PLUGIN_FACTORY_ID);
  if (!factory) {
    entry->deinit();
    set_err(err, cap, "%s has no plugin factory", path);
    dlclose(dl);
    return NULL;
  }
  library *l = calloc(1, sizeof(library));
  l->path = strdup(path);
  l->dl = dl;
  l->entry = entry;
  l->factory = factory;
  l->next = libraries;
  libraries = l;
  return l;
}

int debut_clap_count(const char *path, char *err, int cap) {
  library *l = open_library(path, err, cap);
  return l ? (int)l->factory->get_plugin_count(l->factory) : -1;
}

static int main_channels(const clap_plugin_t *p, int is_input) {
  const clap_plugin_audio_ports_t *ports = p->get_extension(p, CLAP_EXT_AUDIO_PORTS);
  if (!ports) return 2;
  if (ports->count(p, is_input) == 0) return 0;
  clap_audio_port_info_t info;
  memset(&info, 0, sizeof info);
  if (!ports->get(p, 0, is_input, &info)) return 2;
  return info.channel_count >= 2 ? 2 : (int)info.channel_count;
}

static void destroy(instance *in) {
  if (!in) return;
  if (in->plugin) {
    if (in->processing) in->plugin->stop_processing(in->plugin);
    if (in->active) in->plugin->deactivate(in->plugin);
    in->plugin->destroy(in->plugin);
  }
  for (int c = 0; c < 2; c++) {
    free(in->in[c]);
    free(in->out[c]);
  }
  free(in->slots);
  free(in);
}

/* Create plugin `index`, report its parameters through `cb` and, when
 * `rate` > 0, activate it for processing. Returns an opaque instance. */
void *debut_clap_open(const char *path, int index, char *id, char *name, int cap,
                      debut_param_cb cb, void *ctx, double rate, int max_frames, char *err,
                      int errcap) {
  library *l = open_library(path, err, errcap);
  if (!l) return NULL;
  if (index < 0 || (uint32_t)index >= l->factory->get_plugin_count(l->factory)) {
    set_err(err, errcap, "%s has no plugin %d", path, index);
    return NULL;
  }
  const clap_plugin_descriptor_t *d = l->factory->get_plugin_descriptor(l->factory, (uint32_t)index);
  if (!d) {
    set_err(err, errcap, "%s: no descriptor for plugin %d", path, index);
    return NULL;
  }
  if (id) snprintf(id, (size_t)cap, "%s", d->id);
  if (name) snprintf(name, (size_t)cap, "%s", d->name);
  instance *in = calloc(1, sizeof(instance));
  in->plugin = l->factory->create_plugin(l->factory, &host, d->id);
  if (!in->plugin || !in->plugin->init(in->plugin)) {
    destroy(in);
    set_err(err, errcap, "%s failed to start", d->id);
    return NULL;
  }
  in->params = in->plugin->get_extension(in->plugin, CLAP_EXT_PARAMS);
  if (in->params) {
    uint32_t n = in->params->count(in->plugin);
    in->slots = calloc(n + 1, sizeof(param_slot));
    for (uint32_t i = 0; i < n; i++) {
      clap_param_info_t info;
      memset(&info, 0, sizeof info);
      if (!in->params->get_info(in->plugin, i, &info)) continue;
      param_slot *s = &in->slots[in->nslots++];
      s->id = info.id;
      snprintf(s->name, sizeof s->name, "%s", info.name);
      s->sent = info.default_value;
      double v = info.default_value;
      in->params->get_value(in->plugin, info.id, &v);
      s->sent = v;
      if (cb && !(info.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY)))
        cb(ctx, info.name, info.name, (info.flags & CLAP_PARAM_IS_STEPPED) ? "int" : "double", 1,
           info.default_value, info.min_value, info.max_value);
    }
  }
  in->in_ch = main_channels(in->plugin, 1);
  in->out_ch = main_channels(in->plugin, 0);
  if (rate > 0) {
    in->max_frames = max_frames > 0 ? max_frames : 1024;
    for (int c = 0; c < 2; c++) {
      in->in[c] = calloc((size_t)in->max_frames, sizeof(float));
      in->out[c] = calloc((size_t)in->max_frames, sizeof(float));
    }
    if (!in->plugin->activate(in->plugin, rate, 1, (uint32_t)in->max_frames)) {
      destroy(in);
      set_err(err, errcap, "%s refused to activate at %.0f Hz", d->id, rate);
      return NULL;
    }
    in->active = 1;
    if (!in->plugin->start_processing(in->plugin)) {
      destroy(in);
      set_err(err, errcap, "%s refused to start processing", d->id);
      return NULL;
    }
    in->processing = 1;
  }
  return in;
}

/* Queue a parameter change by name; -1 when there is no such parameter. */
int debut_clap_param(void *handle, const char *name, double v) {
  instance *in = handle;
  for (int i = 0; i < in->nslots; i++) {
    param_slot *s = &in->slots[i];
    if (strcmp(s->name, name) != 0) continue;
    if (s->sent == v) return 0;
    if (in->npending == MAX_EVENTS) return -1;
    clap_event_param_value_t *e = &in->pending[in->npending++];
    memset(e, 0, sizeof *e);
    e->header.size = sizeof *e;
    e->header.time = 0;
    e->header.space_id = CLAP_CORE_EVENT_SPACE_ID;
    e->header.type = CLAP_EVENT_PARAM_VALUE;
    e->param_id = s->id;
    e->note_id = -1;
    e->port_index = -1;
    e->channel = -1;
    e->key = -1;
    e->value = v;
    s->sent = v;
    return 0;
  }
  return -1;
}

static uint32_t events_size(const clap_input_events_t *list) {
  return (uint32_t)((instance *)list->ctx)->npending;
}
static const clap_event_header_t *events_get(const clap_input_events_t *list, uint32_t i) {
  instance *in = list->ctx;
  return i < (uint32_t)in->npending ? &in->pending[i].header : NULL;
}
static bool events_push(const clap_output_events_t *list, const clap_event_header_t *e) {
  (void)list, (void)e;
  return true;
}

/* Process interleaved audio in place; -1 on a plugin error. */
int debut_clap_process(void *handle, float *buf, int channels, int frames) {
  instance *in = handle;
  if (!in->max_frames) return -1;
  clap_input_events_t events_in = {in, events_size, events_get};
  clap_output_events_t events_out = {in, events_push};
  for (int start = 0; start < frames; start += in->max_frames) {
    int n = frames - start < in->max_frames ? frames - start : in->max_frames;
    float *chunk = buf + (size_t)start * (size_t)channels;
    for (int f = 0; f < n; f++) {
      float l = chunk[f * channels];
      float r = channels > 1 ? chunk[f * channels + 1] : l;
      if (in->in_ch == 1) {
        in->in[0][f] = 0.5f * (l + r);
      } else {
        in->in[0][f] = l;
        in->in[1][f] = r;
      }
    }
    clap_audio_buffer_t ain = {in->in, NULL, (uint32_t)in->in_ch, 0, 0};
    clap_audio_buffer_t aout = {in->out, NULL, (uint32_t)in->out_ch, 0, 0};
    clap_process_t p;
    memset(&p, 0, sizeof p);
    p.steady_time = in->steady;
    p.frames_count = (uint32_t)n;
    p.audio_inputs = &ain;
    p.audio_inputs_count = in->in_ch ? 1 : 0;
    p.audio_outputs = &aout;
    p.audio_outputs_count = in->out_ch ? 1 : 0;
    p.in_events = &events_in;
    p.out_events = &events_out;
    clap_process_status s = in->plugin->process(in->plugin, &p);
    in->npending = 0;
    in->steady += n;
    if (s == CLAP_PROCESS_ERROR) return -1;
    if (in->out_ch == 0) continue; /* nothing comes out: leave the audio as it was */
    for (int f = 0; f < n; f++) {
      float l = in->out[0][f];
      float r = in->out_ch > 1 ? in->out[1][f] : l;
      chunk[f * channels] = l;
      if (channels > 1) chunk[f * channels + 1] = r;
    }
  }
  return 0;
}

void debut_clap_close(void *handle) { destroy(handle); }
