/* A small OpenFX property store: named, typed, multi-dimensional values. */
#ifndef DEBUT_PROPS_H
#define DEBUT_PROPS_H

#include "ofxCore.h"
#include "ofxProperty.h"

enum prop_type { P_INT, P_DOUBLE, P_STRING, P_POINTER };

typedef struct prop {
  char *name;
  enum prop_type type;
  int count;
  union pv {
    int i;
    double d;
    char *s;
    void *p;
  } * v;
  struct prop *next;
} prop;

typedef struct OfxPropertySetStruct {
  prop *head;
} propset;

propset *props_new(void);
void props_free(propset *ps);
/* Copy every property of `from` into `to` (replacing same-named ones). */
void props_copy(propset *to, const propset *from);
prop *props_find(const propset *ps, const char *name);

/* An empty (zero-dimension) property, for lists a plugin appends to. */
void props_declare(propset *ps, const char *name, enum prop_type type);
void props_set_int(propset *ps, const char *name, int index, int v);
void props_set_double(propset *ps, const char *name, int index, double v);
void props_set_string(propset *ps, const char *name, int index, const char *v);
void props_set_pointer(propset *ps, const char *name, int index, void *v);
int props_int(const propset *ps, const char *name, int index, int fallback);
double props_double(const propset *ps, const char *name, int index, double fallback);
const char *props_string(const propset *ps, const char *name, int index, const char *fallback);
/* Whether a string property holds `value` at any index. */
int props_has_string(const propset *ps, const char *name, const char *value);

extern const OfxPropertySuiteV1 debut_property_suite;

#endif
