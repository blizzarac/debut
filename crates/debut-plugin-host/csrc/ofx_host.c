/* An OpenFX image-effect host for RGBA filters (FX-15).
 *
 * Runs inside the plugin-host helper process, never in the app: a plugin that
 * crashes takes only the helper down. Implements the property, image effect,
 * parameter, memory, multithread and message suites; everything else a plugin
 * asks for is reported missing, which the spec lets it handle.
 *
 * Images are RGBA, premultiplied, at render scale 1, the whole frame as the
 * render window; float when the plugin takes it, else 16- or 8-bit, converted
 * here. The caller's buffers are top-down; OpenFX's are
 * bottom-up, so they are flipped on the way in and out. */

#include <dlfcn.h>
#include <float.h>
#include <limits.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "ofxCore.h"
#include "ofxImageEffect.h"
#include "ofxMemory.h"
#include "ofxMessage.h"
#include "ofxMultiThread.h"
#include "ofxParam.h"
#include "ofxProperty.h"
#include "props.h"

typedef void (*debut_param_cb)(void *ctx, const char *name, const char *label, const char *kind,
                               int dims, double def, double min, double max);

/* ---- objects behind the handles ---- */

typedef struct OfxParamStruct {
  char *name;
  char *type;
  propset *props;
  double v[4];
  char *s;
  struct OfxParamStruct *next;
} param;

typedef struct OfxParamSetStruct {
  param *head;
  propset *props;
} paramset;

typedef struct OfxImageClipStruct {
  char *name;
  propset *props;
  struct OfxImageEffectStruct *owner;
  struct OfxImageClipStruct *next;
} clip;

typedef struct plugin_entry plugin_entry;

typedef struct OfxImageEffectStruct {
  propset *props;
  paramset params;
  clip *clips;
  plugin_entry *plugin;
  /* Instances only: the frame being rendered, bottom-up. */
  int w, h;
  void *src, *dst;
} effect;

typedef struct OfxImageMemoryStruct {
  void *data;
} imagemem;

struct plugin_entry {
  OfxPlugin *plugin;
  int loaded;
  effect *descriptor; /* after kOfxActionDescribe */
  effect *context;    /* after kOfxImageEffectActionDescribeInContext */
  const char *context_name;
  const char *depth; /* the deepest pixel depth it takes */
  int bytes;         /* per component at that depth */
};

typedef struct binary {
  char *path;
  void *dl;
  int count;
  plugin_entry *plugins;
  struct binary *next;
} binary;

static binary *binaries;
static char last_message[512];

static void set_err(char *err, int cap, const char *fmt, ...) {
  if (!err || cap <= 0) return;
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(err, (size_t)cap, fmt, ap);
  va_end(ap);
  if (last_message[0]) {
    size_t n = strlen(err);
    snprintf(err + n, (size_t)cap - n, " (%s)", last_message);
  }
}

static int ok(OfxStatus s) { return s == kOfxStatOK || s == kOfxStatReplyDefault; }

/* ---- params ---- */

static int is_int_type(const char *t) {
  return !strcmp(t, kOfxParamTypeInteger) || !strcmp(t, kOfxParamTypeBoolean) ||
         !strcmp(t, kOfxParamTypeChoice) || !strcmp(t, kOfxParamTypeInteger2D) ||
         !strcmp(t, kOfxParamTypeInteger3D);
}

static int is_string_type(const char *t) {
  return !strcmp(t, kOfxParamTypeString) || !strcmp(t, kOfxParamTypeCustom) ||
         !strcmp(t, kOfxParamTypeStrChoice);
}

static int dims_of(const char *t) {
  if (!strcmp(t, kOfxParamTypeDouble2D) || !strcmp(t, kOfxParamTypeInteger2D)) return 2;
  if (!strcmp(t, kOfxParamTypeDouble3D) || !strcmp(t, kOfxParamTypeInteger3D) ||
      !strcmp(t, kOfxParamTypeRGB))
    return 3;
  if (!strcmp(t, kOfxParamTypeRGBA)) return 4;
  if (!strcmp(t, kOfxParamTypeGroup) || !strcmp(t, kOfxParamTypePage) ||
      !strcmp(t, kOfxParamTypePushButton) || is_string_type(t))
    return 0;
  return 1;
}

static param *param_new(paramset *set, const char *type, const char *name) {
  param *p = calloc(1, sizeof(param));
  p->name = strdup(name);
  p->type = strdup(type);
  p->props = props_new();
  param **tail = &set->head;
  while (*tail) tail = &(*tail)->next;
  *tail = p;
  return p;
}

static param *param_find(paramset *set, const char *name) {
  for (param *p = set->head; p; p = p->next)
    if (!strcmp(p->name, name)) return p;
  return NULL;
}

static void param_defaults(param *p) {
  propset *ps = p->props;
  props_set_string(ps, kOfxPropType, 0, "OfxTypeParameter");
  props_set_string(ps, kOfxParamPropType, 0, p->type);
  props_set_string(ps, kOfxPropName, 0, p->name);
  props_set_string(ps, kOfxPropLabel, 0, p->name);
  props_set_string(ps, kOfxPropShortLabel, 0, p->name);
  props_set_string(ps, kOfxPropLongLabel, 0, p->name);
  props_set_string(ps, kOfxParamPropScriptName, 0, p->name);
  props_set_string(ps, kOfxParamPropHint, 0, "");
  props_set_string(ps, kOfxParamPropParent, 0, "");
  props_set_int(ps, kOfxParamPropSecret, 0, 0);
  props_set_int(ps, kOfxParamPropEnabled, 0, 1);
  props_set_int(ps, kOfxParamPropPersistant, 0, 1);
  props_set_int(ps, kOfxParamPropEvaluateOnChange, 0, 1);
  props_set_int(ps, kOfxParamPropAnimates, 0, 1);
  props_set_pointer(ps, kOfxParamPropDataPtr, 0, NULL);
  int dims = dims_of(p->type);
  if (is_string_type(p->type)) {
    props_set_string(ps, kOfxParamPropDefault, 0, "");
  } else if (is_int_type(p->type)) {
    for (int i = 0; i < dims; i++) {
      props_set_int(ps, kOfxParamPropDefault, i, 0);
      props_set_int(ps, kOfxParamPropMin, i, INT_MIN);
      props_set_int(ps, kOfxParamPropMax, i, INT_MAX);
      props_set_int(ps, kOfxParamPropDisplayMin, i, INT_MIN);
      props_set_int(ps, kOfxParamPropDisplayMax, i, INT_MAX);
    }
  } else if (dims > 0) {
    props_set_string(ps, kOfxParamPropDoubleType, 0, kOfxParamDoubleTypePlain);
    props_set_double(ps, kOfxParamPropIncrement, 0, 1.0);
    props_set_int(ps, kOfxParamPropDigits, 0, 2);
    for (int i = 0; i < dims; i++) {
      props_set_double(ps, kOfxParamPropDefault, i, 0.0);
      props_set_double(ps, kOfxParamPropMin, i, -DBL_MAX);
      props_set_double(ps, kOfxParamPropMax, i, DBL_MAX);
      props_set_double(ps, kOfxParamPropDisplayMin, i, -DBL_MAX);
      props_set_double(ps, kOfxParamPropDisplayMax, i, DBL_MAX);
    }
  }
}

static OfxStatus param_define(OfxParamSetHandle set, const char *type, const char *name,
                              OfxPropertySetHandle *props) {
  if (!set) return kOfxStatErrBadHandle;
  if (param_find(set, name)) return kOfxStatErrExists;
  param *p = param_new(set, type, name);
  param_defaults(p);
  if (props) *props = p->props;
  return kOfxStatOK;
}

static OfxStatus param_get_handle(OfxParamSetHandle set, const char *name, OfxParamHandle *out,
                                  OfxPropertySetHandle *props) {
  if (!set) return kOfxStatErrBadHandle;
  param *p = param_find(set, name);
  if (!p) return kOfxStatErrUnknown;
  if (out) *out = p;
  if (props) *props = p->props;
  return kOfxStatOK;
}

static OfxStatus param_set_props(OfxParamSetHandle set, OfxPropertySetHandle *props) {
  if (!set) return kOfxStatErrBadHandle;
  *props = set->props;
  return kOfxStatOK;
}

static OfxStatus param_props(OfxParamHandle p, OfxPropertySetHandle *props) {
  if (!p) return kOfxStatErrBadHandle;
  *props = p->props;
  return kOfxStatOK;
}

static OfxStatus read_value(param *p, va_list ap) {
  int dims = dims_of(p->type);
  if (is_string_type(p->type)) {
    char **s = va_arg(ap, char **);
    *s = p->s ? p->s : (char *)"";
  } else if (is_int_type(p->type)) {
    for (int i = 0; i < dims; i++) *va_arg(ap, int *) = (int)p->v[i];
  } else {
    for (int i = 0; i < dims; i++) *va_arg(ap, double *) = p->v[i];
  }
  return kOfxStatOK;
}

static OfxStatus write_value(param *p, va_list ap) {
  int dims = dims_of(p->type);
  if (is_string_type(p->type)) {
    const char *s = va_arg(ap, const char *);
    free(p->s);
    p->s = strdup(s ? s : "");
  } else if (is_int_type(p->type)) {
    for (int i = 0; i < dims; i++) p->v[i] = va_arg(ap, int);
  } else {
    for (int i = 0; i < dims; i++) p->v[i] = va_arg(ap, double);
  }
  return kOfxStatOK;
}

static OfxStatus param_get_value(OfxParamHandle p, ...) {
  if (!p) return kOfxStatErrBadHandle;
  va_list ap;
  va_start(ap, p);
  OfxStatus s = read_value(p, ap);
  va_end(ap);
  return s;
}

static OfxStatus param_get_value_at(OfxParamHandle p, OfxTime t, ...) {
  if (!p) return kOfxStatErrBadHandle;
  va_list ap;
  va_start(ap, t);
  OfxStatus s = read_value(p, ap);
  va_end(ap);
  return s;
}

/* Values don't animate inside the host (the app evaluates curves and sends
 * the value per frame), so derivatives are zero and integrals linear. */
static OfxStatus param_get_derivative(OfxParamHandle p, OfxTime t, ...) {
  if (!p) return kOfxStatErrBadHandle;
  (void)t;
  va_list ap;
  va_start(ap, t);
  int dims = dims_of(p->type);
  for (int i = 0; i < dims; i++) *va_arg(ap, double *) = 0.0;
  va_end(ap);
  return kOfxStatOK;
}

static OfxStatus param_get_integral(OfxParamHandle p, OfxTime t1, OfxTime t2, ...) {
  if (!p) return kOfxStatErrBadHandle;
  va_list ap;
  va_start(ap, t2);
  int dims = dims_of(p->type);
  for (int i = 0; i < dims; i++) *va_arg(ap, double *) = p->v[i] * (t2 - t1);
  va_end(ap);
  return kOfxStatOK;
}

static OfxStatus param_set_value(OfxParamHandle p, ...) {
  if (!p) return kOfxStatErrBadHandle;
  va_list ap;
  va_start(ap, p);
  OfxStatus s = write_value(p, ap);
  va_end(ap);
  return s;
}

static OfxStatus param_set_value_at(OfxParamHandle p, OfxTime t, ...) {
  if (!p) return kOfxStatErrBadHandle;
  va_list ap;
  va_start(ap, t);
  OfxStatus s = write_value(p, ap);
  va_end(ap);
  return s;
}

static OfxStatus param_num_keys(OfxParamHandle p, unsigned int *n) {
  if (!p) return kOfxStatErrBadHandle;
  *n = 0;
  return kOfxStatOK;
}
static OfxStatus param_key_time(OfxParamHandle p, unsigned int i, OfxTime *t) {
  (void)p, (void)i, (void)t;
  return kOfxStatErrBadIndex;
}
static OfxStatus param_key_index(OfxParamHandle p, OfxTime t, int dir, int *i) {
  (void)p, (void)t, (void)dir, (void)i;
  return kOfxStatFailed;
}
static OfxStatus param_delete_key(OfxParamHandle p, OfxTime t) {
  (void)p, (void)t;
  return kOfxStatErrBadIndex;
}
static OfxStatus param_delete_all(OfxParamHandle p) {
  (void)p;
  return kOfxStatOK;
}
static OfxStatus param_copy(OfxParamHandle to, OfxParamHandle from, OfxTime off,
                            const OfxRangeD *range) {
  (void)off, (void)range;
  if (!to || !from) return kOfxStatErrBadHandle;
  memcpy(to->v, from->v, sizeof(to->v));
  if (from->s) {
    free(to->s);
    to->s = strdup(from->s);
  }
  return kOfxStatOK;
}
static OfxStatus param_edit_begin(OfxParamSetHandle set, const char *name) {
  (void)set, (void)name;
  return kOfxStatOK;
}
static OfxStatus param_edit_end(OfxParamSetHandle set) {
  (void)set;
  return kOfxStatOK;
}

static const OfxParameterSuiteV1 parameter_suite = {
    param_define,       param_get_handle,     param_set_props,      param_props,
    param_get_value,    param_get_value_at,   param_get_derivative, param_get_integral,
    param_set_value,    param_set_value_at,   param_num_keys,       param_key_time,
    param_key_index,    param_delete_key,     param_delete_all,     param_copy,
    param_edit_begin,   param_edit_end,
};

/* ---- image effect suite ---- */

static clip *clip_find(effect *e, const char *name) {
  for (clip *c = e->clips; c; c = c->next)
    if (!strcmp(c->name, name)) return c;
  return NULL;
}

static clip *clip_new(effect *e, const char *name) {
  clip *c = calloc(1, sizeof(clip));
  c->name = strdup(name);
  c->props = props_new();
  c->owner = e;
  clip **tail = &e->clips;
  while (*tail) tail = &(*tail)->next;
  *tail = c;
  return c;
}

static OfxStatus get_property_set(OfxImageEffectHandle e, OfxPropertySetHandle *props) {
  if (!e) return kOfxStatErrBadHandle;
  *props = e->props;
  return kOfxStatOK;
}

static OfxStatus get_param_set(OfxImageEffectHandle e, OfxParamSetHandle *set) {
  if (!e) return kOfxStatErrBadHandle;
  *set = &e->params;
  return kOfxStatOK;
}

static OfxStatus clip_define(OfxImageEffectHandle e, const char *name,
                             OfxPropertySetHandle *props) {
  if (!e) return kOfxStatErrBadHandle;
  clip *c = clip_find(e, name);
  if (!c) {
    c = clip_new(e, name);
    props_set_string(c->props, kOfxPropType, 0, "OfxTypeClip");
    props_set_string(c->props, kOfxPropName, 0, name);
    props_set_string(c->props, kOfxPropLabel, 0, name);
    props_set_string(c->props, kOfxPropShortLabel, 0, name);
    props_set_string(c->props, kOfxPropLongLabel, 0, name);
    props_set_int(c->props, kOfxImageClipPropOptional, 0, 0);
    props_set_int(c->props, kOfxImageClipPropIsMask, 0, 0);
    props_set_int(c->props, kOfxImageEffectPropTemporalClipAccess, 0, 0);
    props_set_int(c->props, kOfxImageEffectPropSupportsTiles, 0, 1);
    props_set_string(c->props, kOfxImageClipPropFieldExtraction, 0,
                     kOfxImageFieldDoubled);
    props_declare(c->props, kOfxImageEffectPropSupportedComponents, P_STRING);
  }
  if (props) *props = c->props;
  return kOfxStatOK;
}

static OfxStatus clip_get_handle(OfxImageEffectHandle e, const char *name, OfxImageClipHandle *out,
                                 OfxPropertySetHandle *props) {
  if (!e) return kOfxStatErrBadHandle;
  clip *c = clip_find(e, name);
  if (!c) return kOfxStatErrUnknown;
  if (out) *out = c;
  if (props) *props = c->props;
  return kOfxStatOK;
}

static OfxStatus clip_get_props(OfxImageClipHandle c, OfxPropertySetHandle *props) {
  if (!c) return kOfxStatErrBadHandle;
  *props = c->props;
  return kOfxStatOK;
}

static int is_output(const clip *c) { return !strcmp(c->name, kOfxImageEffectOutputClipName); }
static int is_source(const clip *c) { return !strcmp(c->name, kOfxImageEffectSimpleSourceClipName); }

static OfxStatus clip_get_image(OfxImageClipHandle c, OfxTime t, const OfxRectD *region,
                                OfxPropertySetHandle *image) {
  (void)region;
  if (!c) return kOfxStatErrBadHandle;
  effect *e = c->owner;
  void *data = is_output(c) ? e->dst : is_source(c) ? e->src : NULL;
  if (!data) return kOfxStatFailed; /* an unconnected (optional) clip */
  static unsigned long serial;
  char id[32];
  snprintf(id, sizeof id, "%lu", ++serial);
  propset *ps = props_new();
  props_set_string(ps, kOfxPropType, 0, "OfxTypeImage");
  props_set_pointer(ps, kOfxImagePropData, 0, data);
  int bounds[4] = {0, 0, e->w, e->h};
  for (int i = 0; i < 4; i++) {
    props_set_int(ps, kOfxImagePropBounds, i, bounds[i]);
    props_set_int(ps, kOfxImagePropRegionOfDefinition, i, bounds[i]);
  }
  props_set_int(ps, kOfxImagePropRowBytes, 0, e->w * 4 * e->plugin->bytes);
  props_set_string(ps, kOfxImageEffectPropPixelDepth, 0, e->plugin->depth);
  props_set_string(ps, kOfxImageEffectPropComponents, 0, kOfxImageComponentRGBA);
  props_set_string(ps, kOfxImageEffectPropPreMultiplication, 0, kOfxImagePreMultiplied);
  props_set_double(ps, kOfxImagePropPixelAspectRatio, 0, 1.0);
  props_set_double(ps, kOfxImageEffectPropRenderScale, 0, 1.0);
  props_set_double(ps, kOfxImageEffectPropRenderScale, 1, 1.0);
  props_set_string(ps, kOfxImagePropField, 0, kOfxImageFieldNone);
  props_set_string(ps, kOfxImagePropUniqueIdentifier, 0, id);
  props_set_double(ps, kOfxPropTime, 0, t);
  *image = ps;
  return kOfxStatOK;
}

static OfxStatus clip_release_image(OfxPropertySetHandle image) {
  if (!image) return kOfxStatErrBadHandle;
  props_free(image);
  return kOfxStatOK;
}

static OfxStatus clip_rod(OfxImageClipHandle c, OfxTime t, OfxRectD *rod) {
  (void)t;
  if (!c) return kOfxStatErrBadHandle;
  rod->x1 = 0;
  rod->y1 = 0;
  rod->x2 = c->owner->w;
  rod->y2 = c->owner->h;
  return kOfxStatOK;
}

static int effect_abort(OfxImageEffectHandle e) {
  (void)e;
  return 0;
}

static OfxStatus mem_alloc(OfxImageEffectHandle e, size_t n, OfxImageMemoryHandle *out) {
  (void)e;
  imagemem *m = malloc(sizeof(imagemem));
  if (!m) return kOfxStatErrMemory;
  m->data = malloc(n ? n : 1);
  if (!m->data) {
    free(m);
    return kOfxStatErrMemory;
  }
  *out = m;
  return kOfxStatOK;
}
static OfxStatus mem_free(OfxImageMemoryHandle m) {
  if (!m) return kOfxStatErrBadHandle;
  free(m->data);
  free(m);
  return kOfxStatOK;
}
static OfxStatus mem_lock(OfxImageMemoryHandle m, void **p) {
  if (!m) return kOfxStatErrBadHandle;
  *p = m->data;
  return kOfxStatOK;
}
static OfxStatus mem_unlock(OfxImageMemoryHandle m) {
  return m ? kOfxStatOK : kOfxStatErrBadHandle;
}

static const OfxImageEffectSuiteV1 image_effect_suite = {
    get_property_set, get_param_set,      clip_define,  clip_get_handle, clip_get_props,
    clip_get_image,   clip_release_image, clip_rod,     effect_abort,    mem_alloc,
    mem_free,         mem_lock,           mem_unlock,
};

/* ---- memory, threads, messages ---- */

static OfxStatus memory_alloc(void *handle, size_t n, void **out) {
  (void)handle;
  *out = malloc(n ? n : 1);
  return *out ? kOfxStatOK : kOfxStatErrMemory;
}
static OfxStatus memory_free(void *p) {
  free(p);
  return kOfxStatOK;
}
static const OfxMemorySuiteV1 memory_suite = {memory_alloc, memory_free};

static __thread unsigned int thread_index;
static __thread int spawned;

typedef struct {
  OfxThreadFunctionV1 *func;
  unsigned int index, max;
  void *arg;
} job;

static void *run_job(void *p) {
  job *j = p;
  thread_index = j->index;
  spawned = 1;
  j->func(j->index, j->max, j->arg);
  return NULL;
}

static unsigned int cpus(void) {
  long n = sysconf(_SC_NPROCESSORS_ONLN);
  return n > 0 ? (unsigned int)n : 1;
}

static OfxStatus multi_thread(OfxThreadFunctionV1 func, unsigned int n, void *arg) {
  if (n == 0) n = cpus();
  if (n == 1 || spawned) {
    for (unsigned int i = 0; i < n; i++) func(i, n, arg);
    return kOfxStatOK;
  }
  pthread_t *threads = calloc(n, sizeof(pthread_t));
  job *jobs = calloc(n, sizeof(job));
  OfxStatus status = kOfxStatOK;
  unsigned int started = 0;
  for (unsigned int i = 0; i < n; i++) {
    jobs[i] = (job){func, i, n, arg};
    if (pthread_create(&threads[i], NULL, run_job, &jobs[i]) != 0) {
      status = kOfxStatFailed;
      break;
    }
    started++;
  }
  for (unsigned int i = 0; i < started; i++) pthread_join(threads[i], NULL);
  free(threads);
  free(jobs);
  return status;
}
static OfxStatus num_cpus(unsigned int *n) {
  *n = cpus();
  return kOfxStatOK;
}
static OfxStatus multi_thread_index(unsigned int *i) {
  *i = thread_index;
  return kOfxStatOK;
}
static int is_spawned(void) { return spawned; }

static OfxStatus mutex_create(OfxMutexHandle *m, int locks) {
  pthread_mutex_t *mu = malloc(sizeof(pthread_mutex_t));
  pthread_mutexattr_t attr;
  pthread_mutexattr_init(&attr);
  pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_RECURSIVE);
  pthread_mutex_init(mu, &attr);
  pthread_mutexattr_destroy(&attr);
  for (int i = 0; i < locks; i++) pthread_mutex_lock(mu);
  *m = (OfxMutexHandle)mu;
  return kOfxStatOK;
}
static OfxStatus mutex_destroy(const OfxMutexHandle m) {
  if (!m) return kOfxStatErrBadHandle;
  pthread_mutex_destroy((pthread_mutex_t *)m);
  free(m);
  return kOfxStatOK;
}
static OfxStatus mutex_lock(const OfxMutexHandle m) {
  if (!m) return kOfxStatErrBadHandle;
  pthread_mutex_lock((pthread_mutex_t *)m);
  return kOfxStatOK;
}
static OfxStatus mutex_unlock(const OfxMutexHandle m) {
  if (!m) return kOfxStatErrBadHandle;
  pthread_mutex_unlock((pthread_mutex_t *)m);
  return kOfxStatOK;
}
static OfxStatus mutex_try_lock(const OfxMutexHandle m) {
  if (!m) return kOfxStatErrBadHandle;
  return pthread_mutex_trylock((pthread_mutex_t *)m) == 0 ? kOfxStatOK : kOfxStatFailed;
}
static const OfxMultiThreadSuiteV1 multithread_suite = {
    multi_thread, num_cpus,   multi_thread_index, is_spawned,    mutex_create,
    mutex_destroy, mutex_lock, mutex_unlock,       mutex_try_lock,
};

static OfxStatus vmessage(const char *type, const char *id, const char *fmt, va_list ap) {
  char text[sizeof last_message];
  vsnprintf(text, sizeof text, fmt ? fmt : "", ap);
  fprintf(stderr, "ofx %s%s%s: %s\n", type ? type : "", id ? " " : "", id ? id : "", text);
  if (type && (!strcmp(type, kOfxMessageError) || !strcmp(type, kOfxMessageFatal)))
    snprintf(last_message, sizeof last_message, "%s", text);
  if (type && !strcmp(type, kOfxMessageQuestion)) return kOfxStatReplyDefault;
  return kOfxStatOK;
}
static OfxStatus message(void *h, const char *type, const char *id, const char *fmt, ...) {
  (void)h;
  va_list ap;
  va_start(ap, fmt);
  OfxStatus s = vmessage(type, id, fmt, ap);
  va_end(ap);
  return s;
}
static OfxStatus clear_persistent(void *h) {
  (void)h;
  return kOfxStatOK;
}
static const OfxMessageSuiteV1 message_suite_v1 = {message};
static const OfxMessageSuiteV2 message_suite_v2 = {message, message, clear_persistent};

/* ---- the host ---- */

static propset *host_props;

static const void *fetch_suite(OfxPropertySetHandle host, const char *name, int version) {
  (void)host;
  if (!strcmp(name, kOfxPropertySuite) && version == 1) return &debut_property_suite;
  if (!strcmp(name, kOfxImageEffectSuite) && version == 1) return &image_effect_suite;
  if (!strcmp(name, kOfxParameterSuite) && version == 1) return &parameter_suite;
  if (!strcmp(name, kOfxMemorySuite) && version == 1) return &memory_suite;
  if (!strcmp(name, kOfxMultiThreadSuite) && version == 1) return &multithread_suite;
  if (!strcmp(name, kOfxMessageSuite) && version == 1) return &message_suite_v1;
  if (!strcmp(name, kOfxMessageSuite) && version == 2) return &message_suite_v2;
  return NULL;
}

static OfxHost host;

static void host_init(void) {
  if (host_props) return;
  propset *p = host_props = props_new();
  props_set_string(p, kOfxPropType, 0, kOfxTypeImageEffectHost);
  props_set_string(p, kOfxPropName, 0, "dev.debut.app");
  props_set_string(p, kOfxPropLabel, 0, "debut");
  props_set_int(p, kOfxPropAPIVersion, 0, 1);
  props_set_int(p, kOfxPropAPIVersion, 1, 4);
  props_set_int(p, kOfxPropVersion, 0, 0);
  props_set_int(p, kOfxPropVersion, 1, 1);
  props_set_string(p, kOfxPropVersionLabel, 0, "0.1");
  props_set_int(p, kOfxImageEffectHostPropIsBackground, 0, 1);
  props_set_int(p, kOfxImageEffectPropSupportsOverlays, 0, 0);
  props_set_int(p, kOfxImageEffectPropSupportsMultiResolution, 0, 0);
  props_set_int(p, kOfxImageEffectPropSupportsTiles, 0, 0);
  props_set_int(p, kOfxImageEffectPropTemporalClipAccess, 0, 0);
  props_set_string(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);
  props_set_string(p, kOfxImageEffectPropSupportedContexts, 0, kOfxImageEffectContextFilter);
  props_set_string(p, kOfxImageEffectPropSupportedContexts, 1, kOfxImageEffectContextGeneral);
  props_set_string(p, kOfxImageEffectPropSupportedPixelDepths, 0, kOfxBitDepthFloat);
  props_set_string(p, kOfxImageEffectPropSupportedPixelDepths, 1, kOfxBitDepthShort);
  props_set_string(p, kOfxImageEffectPropSupportedPixelDepths, 2, kOfxBitDepthByte);
  props_set_int(p, kOfxImageEffectPropSupportsMultipleClipDepths, 0, 0);
  props_set_int(p, kOfxImageEffectPropSupportsMultipleClipPARs, 0, 0);
  props_set_int(p, kOfxImageEffectPropSetableFrameRate, 0, 0);
  props_set_int(p, kOfxImageEffectPropSetableFielding, 0, 0);
  props_set_int(p, kOfxImageEffectInstancePropSequentialRender, 0, 0);
  props_set_string(p, kOfxImageEffectHostPropNativeOrigin, 0, kOfxHostNativeOriginBottomLeft);
  props_set_int(p, kOfxParamHostPropSupportsCustomInteract, 0, 0);
  props_set_int(p, kOfxParamHostPropSupportsCustomAnimation, 0, 0);
  props_set_int(p, kOfxParamHostPropSupportsStringAnimation, 0, 0);
  props_set_int(p, kOfxParamHostPropSupportsChoiceAnimation, 0, 0);
  props_set_int(p, kOfxParamHostPropSupportsBooleanAnimation, 0, 0);
  props_set_int(p, kOfxParamHostPropMaxParameters, 0, -1);
  props_set_int(p, kOfxParamHostPropMaxPages, 0, 0);
  props_set_int(p, kOfxParamHostPropPageRowColumnCount, 0, 0);
  props_set_int(p, kOfxParamHostPropPageRowColumnCount, 1, 0);
  /* What newer plugins (and the C++ support library) ask about. */
  props_set_int(p, "OfxParamHostPropSupportsStrChoice", 0, 0);
  props_set_int(p, "OfxParamHostPropSupportsStrChoiceAnimation", 0, 0);
  props_set_int(p, "OfxParamHostPropSupportsParametricAnimation", 0, 0);
  props_set_pointer(p, "OfxPropHostOSHandle", 0, NULL);
  props_set_string(p, "OfxImageEffectPropOpenGLRenderSupported", 0, "false");
  props_set_string(p, "OfxImageEffectPropOpenCLRenderSupported", 0, "false");
  props_set_string(p, "OfxImageEffectPropCudaRenderSupported", 0, "false");
  props_set_string(p, "OfxImageEffectPropCudaStreamSupported", 0, "false");
  props_set_string(p, "OfxImageEffectPropMetalRenderSupported", 0, "false");
  props_set_int(p, kOfxImageEffectPropRenderQualityDraft, 0, 0);
  host.host = host_props;
  host.fetchSuite = fetch_suite;
}

static binary *open_binary(const char *path, char *err, int cap) {
  for (binary *b = binaries; b; b = b->next)
    if (!strcmp(b->path, path)) return b;
  host_init();
  void *dl = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  if (!dl) {
    set_err(err, cap, "cannot load %s: %s", path, dlerror());
    return NULL;
  }
  OfxStatus (*set_host)(const OfxHost *) = (OfxStatus(*)(const OfxHost *))dlsym(dl, "OfxSetHost");
  int (*count)(void) = (int (*)(void))dlsym(dl, "OfxGetNumberOfPlugins");
  OfxPlugin *(*get)(int) = (OfxPlugin * (*)(int)) dlsym(dl, "OfxGetPlugin");
  if (!count || !get) {
    set_err(err, cap, "%s is not an OpenFX plugin", path);
    dlclose(dl);
    return NULL;
  }
  if (set_host) set_host(&host);
  binary *b = calloc(1, sizeof(binary));
  b->path = strdup(path);
  b->dl = dl;
  b->count = count();
  if (b->count < 0) b->count = 0;
  b->plugins = calloc((size_t)b->count + 1, sizeof(plugin_entry));
  for (int i = 0; i < b->count; i++) b->plugins[i].plugin = get(i);
  b->next = binaries;
  binaries = b;
  return b;
}

int debut_ofx_count(const char *path, char *err, int cap) {
  binary *b = open_binary(path, err, cap);
  return b ? b->count : -1;
}

static effect *effect_new(plugin_entry *pe, const char *type) {
  effect *e = calloc(1, sizeof(effect));
  e->props = props_new();
  e->params.props = props_new();
  e->plugin = pe;
  props_set_string(e->props, kOfxPropType, 0, type);
  return e;
}

static void effect_free(effect *e) {
  if (!e) return;
  props_free(e->props);
  props_free(e->params.props);
  for (param *p = e->params.head; p;) {
    param *n = p->next;
    props_free(p->props);
    free(p->name);
    free(p->type);
    free(p->s);
    free(p);
    p = n;
  }
  for (clip *c = e->clips; c;) {
    clip *n = c->next;
    props_free(c->props);
    free(c->name);
    free(c);
    c = n;
  }
  free(e->src);
  free(e->dst);
  free(e);
}

static OfxStatus call(plugin_entry *pe, const char *action, const void *handle, propset *in,
                      propset *out) {
  return pe->plugin->mainEntry(action, handle, in, out);
}

static void descriptor_defaults(effect *d, const char *id, const char *path) {
  propset *p = d->props;
  props_set_string(p, kOfxPropLabel, 0, id);
  props_set_string(p, kOfxPropShortLabel, 0, id);
  props_set_string(p, kOfxPropLongLabel, 0, id);
  props_set_pointer(p, kOfxImageEffectPluginPropOverlayInteractV1, 0, NULL);
  /* The bundle directory: three levels up from Contents/<arch>/X.ofx. */
  char bundle[4096];
  snprintf(bundle, sizeof bundle, "%s", path);
  for (int up = 0; up < 3; up++) {
    char *slash = strrchr(bundle, '/');
    if (!slash) break;
    *slash = 0;
  }
  props_set_string(p, kOfxPluginPropFilePath, 0, bundle);
  props_set_string(p, kOfxImageEffectPluginPropGrouping, 0, "");
  props_set_int(p, kOfxImageEffectPluginPropSingleInstance, 0, 0);
  props_set_string(p, kOfxImageEffectPluginRenderThreadSafety, 0,
                   kOfxImageEffectRenderInstanceSafe);
  props_set_int(p, kOfxImageEffectPluginPropHostFrameThreading, 0, 0);
  props_set_int(p, kOfxImageEffectPropSupportsMultiResolution, 0, 1);
  props_set_int(p, kOfxImageEffectPropSupportsTiles, 0, 1);
  props_set_int(p, kOfxImageEffectPropTemporalClipAccess, 0, 0);
  props_set_int(p, kOfxImageEffectPluginPropFieldRenderTwiceAlways, 0, 1);
  props_set_int(p, kOfxImageEffectPropSupportsMultipleClipDepths, 0, 0);
  props_set_int(p, kOfxImageEffectPropSupportsMultipleClipPARs, 0, 0);
  props_declare(p, kOfxImageEffectPropSupportedContexts, P_STRING);
  props_declare(p, kOfxImageEffectPropSupportedPixelDepths, P_STRING);
  props_declare(p, kOfxImageEffectPropClipPreferencesSlaveParam, P_STRING);
}

/* Load, describe and describe in the filter (or general) context; params are
 * reported through `cb`. Returns an opaque descriptor for debut_ofx_instance. */
void *debut_ofx_describe(const char *path, int index, char *id, char *label, int cap,
                         debut_param_cb cb, void *ctx, char *err, int errcap) {
  last_message[0] = 0;
  binary *b = open_binary(path, err, errcap);
  if (!b) return NULL;
  if (index < 0 || index >= b->count || !b->plugins[index].plugin) {
    set_err(err, errcap, "%s has no plugin %d", path, index);
    return NULL;
  }
  plugin_entry *pe = &b->plugins[index];
  OfxPlugin *pl = pe->plugin;
  if (strcmp(pl->pluginApi, kOfxImageEffectPluginApi) != 0) {
    set_err(err, errcap, "%s is a %s plugin, not an image effect", pl->pluginIdentifier,
            pl->pluginApi);
    return NULL;
  }
  if (!pe->loaded) {
    if (pl->setHost) pl->setHost(&host);
    OfxStatus s = call(pe, kOfxActionLoad, NULL, NULL, NULL);
    if (!ok(s)) {
      set_err(err, errcap, "%s failed to load (status %d)", pl->pluginIdentifier, s);
      return NULL;
    }
    pe->loaded = 1;
  }
  if (!pe->descriptor) {
    effect *d = effect_new(pe, kOfxTypeImageEffect);
    descriptor_defaults(d, pl->pluginIdentifier, path);
    OfxStatus s = call(pe, kOfxActionDescribe, d, NULL, NULL);
    if (!ok(s)) {
      effect_free(d);
      set_err(err, errcap, "%s failed to describe itself (status %d)", pl->pluginIdentifier, s);
      return NULL;
    }
    pe->descriptor = d;
  }
  if (!pe->context) {
    effect *d = pe->descriptor;
    const char *context = props_has_string(d->props, kOfxImageEffectPropSupportedContexts,
                                           kOfxImageEffectContextFilter)
                              ? kOfxImageEffectContextFilter
                          : props_has_string(d->props, kOfxImageEffectPropSupportedContexts,
                                             kOfxImageEffectContextGeneral)
                              ? kOfxImageEffectContextGeneral
                              : NULL;
    if (!context) {
      set_err(err, errcap, "%s is not a filter (no filter or general context)",
              pl->pluginIdentifier);
      return NULL;
    }
    /* No list (or an empty one) means any depth. */
    prop *depths = props_find(d->props, kOfxImageEffectPropSupportedPixelDepths);
    const char *key = kOfxImageEffectPropSupportedPixelDepths;
    if (!depths || depths->count == 0 || props_has_string(d->props, key, kOfxBitDepthFloat)) {
      pe->depth = kOfxBitDepthFloat;
      pe->bytes = 4;
    } else if (props_has_string(d->props, key, kOfxBitDepthShort)) {
      pe->depth = kOfxBitDepthShort;
      pe->bytes = 2;
    } else if (props_has_string(d->props, key, kOfxBitDepthByte)) {
      pe->depth = kOfxBitDepthByte;
      pe->bytes = 1;
    } else {
      set_err(err, errcap, "%s takes no 8-bit, 16-bit or float images", pl->pluginIdentifier);
      return NULL;
    }
    effect *c = effect_new(pe, kOfxTypeImageEffect);
    props_copy(c->props, d->props);
    props_set_string(c->props, kOfxImageEffectPropContext, 0, context);
    propset *in = props_new();
    props_set_string(in, kOfxImageEffectPropContext, 0, context);
    OfxStatus s = call(pe, kOfxImageEffectActionDescribeInContext, c, in, NULL);
    props_free(in);
    if (!ok(s)) {
      effect_free(c);
      set_err(err, errcap, "%s failed to describe the %s context (status %d)",
              pl->pluginIdentifier, context, s);
      return NULL;
    }
    if (!clip_find(c, kOfxImageEffectOutputClipName) ||
        !clip_find(c, kOfxImageEffectSimpleSourceClipName)) {
      effect_free(c);
      set_err(err, errcap, "%s has no Source/Output clips", pl->pluginIdentifier);
      return NULL;
    }
    pe->context = c;
    pe->context_name = context;
  }
  if (id) snprintf(id, (size_t)cap, "%s", pl->pluginIdentifier);
  if (label)
    snprintf(label, (size_t)cap, "%s",
             props_string(pe->context->props, kOfxPropLabel, 0, pl->pluginIdentifier));
  if (cb) {
    for (param *p = pe->context->params.head; p; p = p->next) {
      if (props_int(p->props, kOfxParamPropSecret, 0, 0)) continue;
      const char *kind = !strcmp(p->type, kOfxParamTypeBoolean)  ? "bool"
                         : !strcmp(p->type, kOfxParamTypeChoice) ? "choice"
                         : is_int_type(p->type)                  ? "int"
                                                                 : "double";
      int dims = dims_of(p->type);
      if (dims == 0) continue;
      const char *lbl = props_string(p->props, kOfxPropLabel, 0, p->name);
      double lo = props_double(p->props, kOfxParamPropDisplayMin, 0, -DBL_MAX);
      double hi = props_double(p->props, kOfxParamPropDisplayMax, 0, DBL_MAX);
      if (lo <= -DBL_MAX || lo <= INT_MIN) lo = props_double(p->props, kOfxParamPropMin, 0, lo);
      if (hi >= DBL_MAX || hi >= INT_MAX) hi = props_double(p->props, kOfxParamPropMax, 0, hi);
      if (!strcmp(p->type, kOfxParamTypeChoice)) {
        prop *opts = props_find(p->props, kOfxParamPropChoiceOption);
        lo = 0;
        hi = opts && opts->count > 0 ? opts->count - 1 : 0;
      } else if (!strcmp(p->type, kOfxParamTypeBoolean)) {
        lo = 0;
        hi = 1;
      }
      for (int i = 0; i < dims; i++) {
        char name[256];
        if (dims == 1)
          snprintf(name, sizeof name, "%s", p->name);
        else
          snprintf(name, sizeof name, "%s[%d]", p->name, i);
        cb(ctx, name, lbl, kind, dims, props_double(p->props, kOfxParamPropDefault, i, 0.0), lo,
           hi);
      }
    }
  }
  return pe;
}

static void set_clip_prefs(clip *c, const char *depth, double fps) {
  propset *p = c->props;
  props_set_string(p, kOfxImageEffectPropPixelDepth, 0, depth);
  props_set_string(p, kOfxImageEffectPropComponents, 0, kOfxImageComponentRGBA);
  props_set_string(p, kOfxImageClipPropUnmappedPixelDepth, 0, depth);
  props_set_string(p, kOfxImageClipPropUnmappedComponents, 0, kOfxImageComponentRGBA);
  props_set_string(p, kOfxImageEffectPropPreMultiplication, 0, kOfxImagePreMultiplied);
  props_set_double(p, kOfxImagePropPixelAspectRatio, 0, 1.0);
  props_set_double(p, kOfxImageEffectPropFrameRate, 0, fps);
  props_set_double(p, kOfxImageEffectPropUnmappedFrameRate, 0, fps);
  props_set_double(p, kOfxImageEffectPropFrameRange, 0, 0.0);
  props_set_double(p, kOfxImageEffectPropFrameRange, 1, 1e7);
  props_set_double(p, kOfxImageEffectPropUnmappedFrameRange, 0, 0.0);
  props_set_double(p, kOfxImageEffectPropUnmappedFrameRange, 1, 1e7);
  props_set_string(p, kOfxImageClipPropFieldOrder, 0, kOfxImageFieldNone);
  props_set_int(p, kOfxImageClipPropConnected, 0, is_output(c) || is_source(c));
  props_set_int(p, kOfxImageClipPropContinuousSamples, 0, 0);
}

static void set_size(effect *e, int w, int h, double fps) {
  propset *p = e->props;
  double size[2] = {w, h};
  for (int i = 0; i < 2; i++) {
    props_set_double(p, kOfxImageEffectPropProjectSize, i, size[i]);
    props_set_double(p, kOfxImageEffectPropProjectExtent, i, size[i]);
    props_set_double(p, kOfxImageEffectPropProjectOffset, i, 0.0);
  }
  props_set_double(p, kOfxImageEffectPropProjectPixelAspectRatio, 0, 1.0);
  props_set_double(p, kOfxImageEffectPropFrameRate, 0, fps);
  for (clip *c = e->clips; c; c = c->next) set_clip_prefs(c, e->plugin->depth, fps);
}

void *debut_ofx_instance(void *descriptor, double fps, char *err, int errcap) {
  last_message[0] = 0;
  plugin_entry *pe = descriptor;
  effect *d = pe->context;
  effect *e = effect_new(pe, kOfxTypeImageEffectInstance);
  props_copy(e->props, d->props);
  props_set_string(e->props, kOfxPropType, 0, kOfxTypeImageEffectInstance);
  props_set_string(e->props, kOfxImageEffectPropContext, 0, pe->context_name);
  props_set_int(e->props, kOfxPropIsInteractive, 0, 0);
  props_set_double(e->props, kOfxImageEffectInstancePropEffectDuration, 0, 1e7);
  props_set_int(e->props, kOfxImageEffectInstancePropSequentialRender, 0, 0);
  props_set_pointer(e->props, kOfxPropInstanceData, 0, NULL);
  props_set_pointer(e->props, "OfxImageEffectPropPluginHandle", 0, pe->plugin);
  props_set_int(e->props, "OfxImageEffectPropOpenGLEnabled", 0, 0);
  props_set_int(e->props, "OfxImageEffectPropOpenCLEnabled", 0, 0);
  props_set_int(e->props, "OfxImageEffectPropCudaEnabled", 0, 0);
  props_set_int(e->props, "OfxImageEffectPropMetalEnabled", 0, 0);
  props_set_pointer(e->props, "OfxImageEffectPropOpenCLCommandQueue", 0, NULL);
  props_set_pointer(e->props, "OfxImageEffectPropCudaStream", 0, NULL);
  props_set_pointer(e->props, "OfxImageEffectPropMetalCommandQueue", 0, NULL);
  props_copy(e->params.props, d->params.props);
  for (param *dp = d->params.head; dp; dp = dp->next) {
    param *p = param_new(&e->params, dp->type, dp->name);
    props_copy(p->props, dp->props);
    if (is_string_type(p->type)) {
      p->s = strdup(props_string(p->props, kOfxParamPropDefault, 0, ""));
    } else {
      for (int i = 0; i < dims_of(p->type); i++)
        p->v[i] = props_double(p->props, kOfxParamPropDefault, i, 0.0);
    }
  }
  for (clip *dc = d->clips; dc; dc = dc->next) {
    clip *c = clip_new(e, dc->name);
    props_copy(c->props, dc->props);
  }
  set_size(e, 1, 1, fps);
  OfxStatus s = call(pe, kOfxActionCreateInstance, e, NULL, NULL);
  if (!ok(s)) {
    effect_free(e);
    set_err(err, errcap, "%s failed to create an instance (status %d)",
            pe->plugin->pluginIdentifier, s);
    return NULL;
  }
  return e;
}

/* Set component `comp` of a numeric parameter; -1 when there is no such parameter. */
int debut_ofx_param(void *instance, const char *name, int comp, double v) {
  effect *e = instance;
  param *p = param_find(&e->params, name);
  if (!p || comp < 0 || comp >= dims_of(p->type)) return -1;
  p->v[comp] = is_int_type(p->type) ? (double)(long)(v + (v < 0 ? -0.5 : 0.5)) : v;
  return 0;
}

/* Float components into `bytes`-wide ones (4: as they are), and back. */
static void pack(int bytes, const float *in, void *out, size_t n) {
  if (bytes == 4) {
    memcpy(out, in, n * sizeof(float));
    return;
  }
  float scale = bytes == 2 ? 65535.0f : 255.0f;
  for (size_t i = 0; i < n; i++) {
    float v = in[i] < 0 ? 0 : in[i] > 1 ? 1 : in[i];
    unsigned int q = (unsigned int)(v * scale + 0.5f);
    if (bytes == 2)
      ((unsigned short *)out)[i] = (unsigned short)q;
    else
      ((unsigned char *)out)[i] = (unsigned char)q;
  }
}

static void unpack(int bytes, const void *in, float *out, size_t n) {
  if (bytes == 4) {
    memcpy(out, in, n * sizeof(float));
    return;
  }
  for (size_t i = 0; i < n; i++)
    out[i] = bytes == 2 ? ((const unsigned short *)in)[i] / 65535.0f
                        : ((const unsigned char *)in)[i] / 255.0f;
}

int debut_ofx_render(void *instance, double time, double fps, int w, int h, float *rgba,
                     char *err, int errcap) {
  last_message[0] = 0;
  effect *e = instance;
  plugin_entry *pe = e->plugin;
  size_t row = (size_t)w * 4;
  size_t row_bytes = row * (size_t)pe->bytes;
  if (e->w != w || e->h != h) {
    free(e->src);
    free(e->dst);
    e->src = malloc(row_bytes * (size_t)h);
    e->dst = malloc(row_bytes * (size_t)h);
    e->w = w;
    e->h = h;
  }
  set_size(e, w, h, fps);
  for (int y = 0; y < h; y++)
    pack(pe->bytes, rgba + row * (size_t)y, (char *)e->src + row_bytes * (size_t)(h - 1 - y), row);
  memset(e->dst, 0, row_bytes * (size_t)h);

  propset *in = props_new();
  props_set_double(in, kOfxPropTime, 0, time);
  props_set_string(in, kOfxImageEffectPropFieldToRender, 0, kOfxImageFieldNone);
  int window[4] = {0, 0, w, h};
  for (int i = 0; i < 4; i++) props_set_int(in, kOfxImageEffectPropRenderWindow, i, window[i]);
  props_set_double(in, kOfxImageEffectPropRenderScale, 0, 1.0);
  props_set_double(in, kOfxImageEffectPropRenderScale, 1, 1.0);
  props_set_int(in, kOfxImageEffectPropSequentialRenderStatus, 0, 0);
  props_set_int(in, kOfxImageEffectPropInteractiveRenderStatus, 0, 0);
  props_set_int(in, kOfxImageEffectPropRenderQualityDraft, 0, 0);
  /* No GPU APIs are offered: render on the CPU. */
  props_set_int(in, "OfxImageEffectPropOpenGLEnabled", 0, 0);
  props_set_int(in, "OfxImageEffectPropOpenCLEnabled", 0, 0);
  props_set_int(in, "OfxImageEffectPropCudaEnabled", 0, 0);
  props_set_int(in, "OfxImageEffectPropMetalEnabled", 0, 0);
  props_set_pointer(in, "OfxImageEffectPropOpenCLCommandQueue", 0, NULL);
  props_set_pointer(in, "OfxImageEffectPropCudaStream", 0, NULL);
  props_set_pointer(in, "OfxImageEffectPropMetalCommandQueue", 0, NULL);
  propset *seq = props_new();
  props_set_double(seq, kOfxImageEffectPropFrameRange, 0, time);
  props_set_double(seq, kOfxImageEffectPropFrameRange, 1, time);
  props_set_double(seq, kOfxImageEffectPropFrameStep, 0, 1.0);
  props_set_int(seq, kOfxPropIsInteractive, 0, 0);
  props_set_double(seq, kOfxImageEffectPropRenderScale, 0, 1.0);
  props_set_double(seq, kOfxImageEffectPropRenderScale, 1, 1.0);
  props_set_int(seq, kOfxImageEffectPropSequentialRenderStatus, 0, 0);
  props_set_int(seq, kOfxImageEffectPropInteractiveRenderStatus, 0, 0);

  OfxStatus s = call(pe, kOfxImageEffectActionBeginSequenceRender, e, seq, NULL);
  if (ok(s)) {
    s = call(pe, kOfxImageEffectActionRender, e, in, NULL);
    call(pe, kOfxImageEffectActionEndSequenceRender, e, seq, NULL);
  }
  props_free(in);
  props_free(seq);
  if (!ok(s)) {
    set_err(err, errcap, "%s failed to render (status %d)", pe->plugin->pluginIdentifier, s);
    return -1;
  }
  for (int y = 0; y < h; y++)
    unpack(pe->bytes, (const char *)e->dst + row_bytes * (size_t)(h - 1 - y),
           rgba + row * (size_t)y, row);
  return 0;
}

void debut_ofx_destroy(void *instance) {
  effect *e = instance;
  if (!e) return;
  call(e->plugin, kOfxActionDestroyInstance, e, NULL, NULL);
  effect_free(e);
}
