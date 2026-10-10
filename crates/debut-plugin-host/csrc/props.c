#include "props.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

propset *props_new(void) { return calloc(1, sizeof(propset)); }

static void prop_clear_values(prop *p) {
  if (p->type == P_STRING)
    for (int i = 0; i < p->count; i++) free(p->v[i].s);
  free(p->v);
  p->v = NULL;
  p->count = 0;
}

void props_free(propset *ps) {
  if (!ps) return;
  prop *p = ps->head;
  while (p) {
    prop *n = p->next;
    prop_clear_values(p);
    free(p->name);
    free(p);
    p = n;
  }
  free(ps);
}

prop *props_find(const propset *ps, const char *name) {
  if (!ps || !name) return NULL;
  for (prop *p = ps->head; p; p = p->next)
    if (strcmp(p->name, name) == 0) return p;
  return NULL;
}

/* The property `name`, created with `type` if missing and grown to hold
 * `index`. A numeric property set with the other numeric type keeps its own
 * type (the value converts); any other mismatch replaces it. */
static prop *slot(propset *ps, const char *name, enum prop_type type, int index) {
  prop *p = props_find(ps, name);
  if (!p) {
    p = calloc(1, sizeof(prop));
    p->name = strdup(name);
    p->type = type;
    p->next = ps->head;
    ps->head = p;
  } else if (p->type != type && !((p->type == P_INT || p->type == P_DOUBLE) &&
                                  (type == P_INT || type == P_DOUBLE))) {
    prop_clear_values(p);
    p->type = type;
  }
  if (index >= p->count) {
    p->v = realloc(p->v, sizeof(*p->v) * (size_t)(index + 1));
    memset(p->v + p->count, 0, sizeof(*p->v) * (size_t)(index + 1 - p->count));
    p->count = index + 1;
  }
  return p;
}

void props_declare(propset *ps, const char *name, enum prop_type type) {
  if (props_find(ps, name)) return;
  prop *p = calloc(1, sizeof(prop));
  p->name = strdup(name);
  p->type = type;
  p->next = ps->head;
  ps->head = p;
}

void props_set_int(propset *ps, const char *name, int index, int v) {
  prop *p = slot(ps, name, P_INT, index);
  if (p->type == P_DOUBLE)
    p->v[index].d = v;
  else
    p->v[index].i = v;
}

void props_set_double(propset *ps, const char *name, int index, double v) {
  prop *p = slot(ps, name, P_DOUBLE, index);
  if (p->type == P_INT)
    p->v[index].i = (int)v;
  else
    p->v[index].d = v;
}

void props_set_string(propset *ps, const char *name, int index, const char *v) {
  prop *p = slot(ps, name, P_STRING, index);
  free(p->v[index].s);
  p->v[index].s = strdup(v ? v : "");
}

void props_set_pointer(propset *ps, const char *name, int index, void *v) {
  slot(ps, name, P_POINTER, index)->v[index].p = v;
}

void props_copy(propset *to, const propset *from) {
  if (!from) return;
  for (prop *p = from->head; p; p = p->next) {
    prop *q = props_find(to, p->name);
    if (q) prop_clear_values(q);
    if (p->count == 0) props_declare(to, p->name, p->type);
    for (int i = p->count - 1; i >= 0; i--) {
      switch (p->type) {
        case P_INT: props_set_int(to, p->name, i, p->v[i].i); break;
        case P_DOUBLE: props_set_double(to, p->name, i, p->v[i].d); break;
        case P_STRING: props_set_string(to, p->name, i, p->v[i].s); break;
        case P_POINTER: props_set_pointer(to, p->name, i, p->v[i].p); break;
      }
    }
  }
}

int props_int(const propset *ps, const char *name, int index, int fallback) {
  prop *p = props_find(ps, name);
  if (!p || index >= p->count) return fallback;
  if (p->type == P_INT) return p->v[index].i;
  if (p->type == P_DOUBLE) return (int)p->v[index].d;
  return fallback;
}

double props_double(const propset *ps, const char *name, int index, double fallback) {
  prop *p = props_find(ps, name);
  if (!p || index >= p->count) return fallback;
  if (p->type == P_DOUBLE) return p->v[index].d;
  if (p->type == P_INT) return p->v[index].i;
  return fallback;
}

const char *props_string(const propset *ps, const char *name, int index, const char *fallback) {
  prop *p = props_find(ps, name);
  if (!p || p->type != P_STRING || index >= p->count) return fallback;
  return p->v[index].s;
}

int props_has_string(const propset *ps, const char *name, const char *value) {
  prop *p = props_find(ps, name);
  if (!p || p->type != P_STRING) return 0;
  for (int i = 0; i < p->count; i++)
    if (strcmp(p->v[i].s, value) == 0) return 1;
  return 0;
}

/* ---- the suite ---- */

static OfxStatus set_pointer(OfxPropertySetHandle h, const char *n, int i, void *v) {
  if (!h) return kOfxStatErrBadHandle;
  if (i < 0) return kOfxStatErrBadIndex;
  props_set_pointer(h, n, i, v);
  return kOfxStatOK;
}
static OfxStatus set_string(OfxPropertySetHandle h, const char *n, int i, const char *v) {
  if (!h) return kOfxStatErrBadHandle;
  if (i < 0) return kOfxStatErrBadIndex;
  props_set_string(h, n, i, v);
  return kOfxStatOK;
}
static OfxStatus set_double(OfxPropertySetHandle h, const char *n, int i, double v) {
  if (!h) return kOfxStatErrBadHandle;
  if (i < 0) return kOfxStatErrBadIndex;
  props_set_double(h, n, i, v);
  return kOfxStatOK;
}
static OfxStatus set_int(OfxPropertySetHandle h, const char *n, int i, int v) {
  if (!h) return kOfxStatErrBadHandle;
  if (i < 0) return kOfxStatErrBadIndex;
  props_set_int(h, n, i, v);
  return kOfxStatOK;
}

/* Setting N values replaces the property, so a shorter list shrinks it. */
static void reset_named(propset *ps, const char *n) {
  prop *p = props_find(ps, n);
  if (p) prop_clear_values(p);
}

static OfxStatus set_pointer_n(OfxPropertySetHandle h, const char *n, int c, void *const *v) {
  if (!h) return kOfxStatErrBadHandle;
  reset_named(h, n);
  for (int i = c - 1; i >= 0; i--) props_set_pointer(h, n, i, v[i]);
  return kOfxStatOK;
}
static OfxStatus set_string_n(OfxPropertySetHandle h, const char *n, int c, const char *const *v) {
  if (!h) return kOfxStatErrBadHandle;
  reset_named(h, n);
  for (int i = c - 1; i >= 0; i--) props_set_string(h, n, i, v[i]);
  return kOfxStatOK;
}
static OfxStatus set_double_n(OfxPropertySetHandle h, const char *n, int c, const double *v) {
  if (!h) return kOfxStatErrBadHandle;
  reset_named(h, n);
  for (int i = c - 1; i >= 0; i--) props_set_double(h, n, i, v[i]);
  return kOfxStatOK;
}
static OfxStatus set_int_n(OfxPropertySetHandle h, const char *n, int c, const int *v) {
  if (!h) return kOfxStatErrBadHandle;
  reset_named(h, n);
  for (int i = c - 1; i >= 0; i--) props_set_int(h, n, i, v[i]);
  return kOfxStatOK;
}

static OfxStatus lookup(OfxPropertySetHandle h, const char *n, int i, prop **out) {
  if (!h) return kOfxStatErrBadHandle;
  prop *p = props_find(h, n);
  if (!p) {
    if (getenv("DEBUT_OFX_TRACE")) fprintf(stderr, "ofx: unknown property %s\n", n);
    return kOfxStatErrUnknown;
  }
  if (i < 0 || i >= p->count) return kOfxStatErrBadIndex;
  *out = p;
  return kOfxStatOK;
}

static OfxStatus get_pointer(OfxPropertySetHandle h, const char *n, int i, void **v) {
  prop *p;
  OfxStatus s = lookup(h, n, i, &p);
  if (s != kOfxStatOK) return s;
  if (p->type != P_POINTER) return kOfxStatErrValue;
  *v = p->v[i].p;
  return kOfxStatOK;
}
static OfxStatus get_string(OfxPropertySetHandle h, const char *n, int i, char **v) {
  prop *p;
  OfxStatus s = lookup(h, n, i, &p);
  if (s != kOfxStatOK) return s;
  if (p->type != P_STRING) return kOfxStatErrValue;
  *v = p->v[i].s;
  return kOfxStatOK;
}
static OfxStatus get_double(OfxPropertySetHandle h, const char *n, int i, double *v) {
  prop *p;
  OfxStatus s = lookup(h, n, i, &p);
  if (s != kOfxStatOK) return s;
  if (p->type == P_DOUBLE)
    *v = p->v[i].d;
  else if (p->type == P_INT)
    *v = p->v[i].i;
  else
    return kOfxStatErrValue;
  return kOfxStatOK;
}
static OfxStatus get_int(OfxPropertySetHandle h, const char *n, int i, int *v) {
  prop *p;
  OfxStatus s = lookup(h, n, i, &p);
  if (s != kOfxStatOK) return s;
  if (p->type == P_INT)
    *v = p->v[i].i;
  else if (p->type == P_DOUBLE)
    *v = (int)p->v[i].d;
  else
    return kOfxStatErrValue;
  return kOfxStatOK;
}
static OfxStatus get_pointer_n(OfxPropertySetHandle h, const char *n, int c, void **v) {
  for (int i = 0; i < c; i++) {
    OfxStatus s = get_pointer(h, n, i, &v[i]);
    if (s != kOfxStatOK) return s;
  }
  return kOfxStatOK;
}
static OfxStatus get_string_n(OfxPropertySetHandle h, const char *n, int c, char **v) {
  for (int i = 0; i < c; i++) {
    OfxStatus s = get_string(h, n, i, &v[i]);
    if (s != kOfxStatOK) return s;
  }
  return kOfxStatOK;
}
static OfxStatus get_double_n(OfxPropertySetHandle h, const char *n, int c, double *v) {
  for (int i = 0; i < c; i++) {
    OfxStatus s = get_double(h, n, i, &v[i]);
    if (s != kOfxStatOK) return s;
  }
  return kOfxStatOK;
}
static OfxStatus get_int_n(OfxPropertySetHandle h, const char *n, int c, int *v) {
  for (int i = 0; i < c; i++) {
    OfxStatus s = get_int(h, n, i, &v[i]);
    if (s != kOfxStatOK) return s;
  }
  return kOfxStatOK;
}
static OfxStatus reset(OfxPropertySetHandle h, const char *n) {
  if (!h) return kOfxStatErrBadHandle;
  prop *p = props_find(h, n);
  if (!p) return kOfxStatErrUnknown;
  for (int i = 0; i < p->count; i++) {
    if (p->type == P_STRING) {
      free(p->v[i].s);
      p->v[i].s = strdup("");
    } else {
      memset(&p->v[i], 0, sizeof(p->v[i]));
    }
  }
  return kOfxStatOK;
}
static OfxStatus dimension(OfxPropertySetHandle h, const char *n, int *count) {
  if (!h) return kOfxStatErrBadHandle;
  prop *p = props_find(h, n);
  if (!p) {
    if (getenv("DEBUT_OFX_TRACE")) fprintf(stderr, "ofx: unknown property %s\n", n);
    return kOfxStatErrUnknown;
  }
  *count = p->count;
  return kOfxStatOK;
}

const OfxPropertySuiteV1 debut_property_suite = {
    set_pointer,   set_string,   set_double,   set_int,      set_pointer_n, set_string_n,
    set_double_n,  set_int_n,    get_pointer,  get_string,   get_double,    get_int,
    get_pointer_n, get_string_n, get_double_n, get_int_n,    reset,         dimension,
};
