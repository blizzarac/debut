/* Test plugin: a CLAP stereo gain with one "Gain" parameter (linear) and a
 * hidden "Crash" parameter that aborts the process when set. */

#include <stdlib.h>
#include <string.h>

#include "clap/entry.h"
#include "clap/events.h"
#include "clap/ext/audio-ports.h"
#include "clap/ext/params.h"
#include "clap/factory/plugin-factory.h"
#include "clap/plugin.h"
#include "clap/process.h"

typedef struct {
  clap_plugin_t plugin;
  double gain;
} gain_t;

static const char *features[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, NULL};

static const clap_plugin_descriptor_t desc = {
    CLAP_VERSION_INIT, "dev.debut.test.gain", "Test Gain", "debut", "", "", "", "1.0",
    "Scales audio",    features,
};

static bool init(const clap_plugin_t *p) { (void)p; return true; }
static void destroy(const clap_plugin_t *p) { free(p->plugin_data); }
static bool activate(const clap_plugin_t *p, double sr, uint32_t a, uint32_t b) {
  (void)p, (void)sr, (void)a, (void)b;
  return true;
}
static void deactivate(const clap_plugin_t *p) { (void)p; }
static bool start(const clap_plugin_t *p) { (void)p; return true; }
static void stop(const clap_plugin_t *p) { (void)p; }
static void reset(const clap_plugin_t *p) { (void)p; }

static void handle_events(gain_t *g, const clap_input_events_t *in) {
  uint32_t n = in->size(in);
  for (uint32_t i = 0; i < n; i++) {
    const clap_event_header_t *h = in->get(in, i);
    if (h->space_id != CLAP_CORE_EVENT_SPACE_ID || h->type != CLAP_EVENT_PARAM_VALUE) continue;
    const clap_event_param_value_t *e = (const clap_event_param_value_t *)h;
    if (e->param_id == 0) g->gain = e->value;
    if (e->param_id == 1 && e->value > 0) abort();
  }
}

static clap_process_status process(const clap_plugin_t *p, const clap_process_t *pr) {
  gain_t *g = p->plugin_data;
  handle_events(g, pr->in_events);
  for (uint32_t c = 0; c < 2; c++)
    for (uint32_t f = 0; f < pr->frames_count; f++)
      pr->audio_outputs[0].data32[c][f] = pr->audio_inputs[0].data32[c][f] * (float)g->gain;
  return CLAP_PROCESS_CONTINUE;
}

static uint32_t ports_count(const clap_plugin_t *p, bool in) { (void)p, (void)in; return 1; }
static bool ports_get(const clap_plugin_t *p, uint32_t i, bool in, clap_audio_port_info_t *info) {
  (void)p, (void)in;
  if (i != 0) return false;
  memset(info, 0, sizeof *info);
  info->id = 0;
  strcpy(info->name, "main");
  info->flags = CLAP_AUDIO_PORT_IS_MAIN;
  info->channel_count = 2;
  info->port_type = CLAP_PORT_STEREO;
  info->in_place_pair = CLAP_INVALID_ID;
  return true;
}
static const clap_plugin_audio_ports_t ports = {ports_count, ports_get};

static uint32_t params_count(const clap_plugin_t *p) { (void)p; return 2; }
static bool params_info(const clap_plugin_t *p, uint32_t i, clap_param_info_t *info) {
  (void)p;
  memset(info, 0, sizeof *info);
  info->id = i;
  if (i == 0) {
    strcpy(info->name, "Gain");
    info->flags = CLAP_PARAM_IS_AUTOMATABLE;
    info->min_value = 0;
    info->max_value = 4;
    info->default_value = 1;
    return true;
  }
  if (i == 1) {
    strcpy(info->name, "Crash");
    info->flags = CLAP_PARAM_IS_HIDDEN;
    info->max_value = 1;
    return true;
  }
  return false;
}
static bool params_value(const clap_plugin_t *p, clap_id id, double *v) {
  gain_t *g = p->plugin_data;
  if (id == 0) *v = g->gain;
  else if (id == 1) *v = 0;
  else return false;
  return true;
}
static bool params_to_text(const clap_plugin_t *p, clap_id id, double v, char *out, uint32_t n) {
  (void)p, (void)id, (void)v, (void)out, (void)n;
  return false;
}
static bool params_from_text(const clap_plugin_t *p, clap_id id, const char *t, double *v) {
  (void)p, (void)id, (void)t, (void)v;
  return false;
}
static void params_flush(const clap_plugin_t *p, const clap_input_events_t *in,
                         const clap_output_events_t *out) {
  (void)out;
  handle_events(p->plugin_data, in);
}
static const clap_plugin_params_t params = {params_count,   params_info,      params_value,
                                            params_to_text, params_from_text, params_flush};

static const void *get_extension(const clap_plugin_t *p, const char *id) {
  (void)p;
  if (!strcmp(id, CLAP_EXT_AUDIO_PORTS)) return &ports;
  if (!strcmp(id, CLAP_EXT_PARAMS)) return &params;
  return NULL;
}
static void on_main_thread(const clap_plugin_t *p) { (void)p; }

static uint32_t factory_count(const clap_plugin_factory_t *f) { (void)f; return 1; }
static const clap_plugin_descriptor_t *factory_desc(const clap_plugin_factory_t *f, uint32_t i) {
  (void)f;
  return i == 0 ? &desc : NULL;
}
static const clap_plugin_t *factory_create(const clap_plugin_factory_t *f, const clap_host_t *h,
                                           const char *id) {
  (void)f, (void)h;
  if (strcmp(id, desc.id) != 0) return NULL;
  gain_t *g = calloc(1, sizeof(gain_t));
  g->gain = 1.0;
  g->plugin = (clap_plugin_t){&desc,      g,     init,    destroy,       activate,
                              deactivate, start, stop,    reset,         process,
                              get_extension, on_main_thread};
  return &g->plugin;
}
static const clap_plugin_factory_t factory = {factory_count, factory_desc, factory_create};

static bool entry_init(const char *path) { (void)path; return true; }
static void entry_deinit(void) {}
static const void *entry_factory(const char *id) {
  return !strcmp(id, CLAP_PLUGIN_FACTORY_ID) ? &factory : NULL;
}

CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit,
                                                    entry_factory};
