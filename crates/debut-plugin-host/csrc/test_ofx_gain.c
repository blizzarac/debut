/* Test plugin: an OpenFX filter that scales RGB by "gain". With "bottom" set
 * it only touches the bottom half of the frame in OpenFX's own (bottom-up)
 * coordinates, so tests can check the host gets image orientation right.
 * "crash" > 0 aborts the process, to test that the app survives it. */

#include <stdlib.h>
#include <string.h>

#include "ofxCore.h"
#include "ofxImageEffect.h"
#include "ofxParam.h"
#include "ofxProperty.h"

static OfxHost *host;
static const OfxPropertySuiteV1 *props;
static const OfxImageEffectSuiteV1 *fx;
static const OfxParameterSuiteV1 *params;

static void set_host(OfxHost *h) { host = h; }

static OfxStatus describe(OfxImageEffectHandle d) {
  OfxPropertySetHandle p;
  fx->getPropertySet(d, &p);
  props->propSetString(p, kOfxPropLabel, 0, "Test Gain");
  props->propSetString(p, kOfxImageEffectPluginPropGrouping, 0, "debut tests");
  props->propSetString(p, kOfxImageEffectPropSupportedContexts, 0, kOfxImageEffectContextFilter);
  props->propSetString(p, kOfxImageEffectPropSupportedPixelDepths, 0, kOfxBitDepthFloat);
  return kOfxStatOK;
}

static OfxStatus describe_in_context(OfxImageEffectHandle d) {
  OfxPropertySetHandle p;
  fx->clipDefine(d, kOfxImageEffectSimpleSourceClipName, &p);
  props->propSetString(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);
  fx->clipDefine(d, kOfxImageEffectOutputClipName, &p);
  props->propSetString(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);
  OfxParamSetHandle set;
  fx->getParamSet(d, &set);
  params->paramDefine(set, kOfxParamTypeDouble, "gain", &p);
  props->propSetString(p, kOfxPropLabel, 0, "Gain");
  props->propSetDouble(p, kOfxParamPropDefault, 0, 1.0);
  props->propSetDouble(p, kOfxParamPropDisplayMin, 0, 0.0);
  props->propSetDouble(p, kOfxParamPropDisplayMax, 0, 4.0);
  params->paramDefine(set, kOfxParamTypeBoolean, "bottom", &p);
  props->propSetString(p, kOfxPropLabel, 0, "Bottom half only");
  props->propSetInt(p, kOfxParamPropDefault, 0, 0);
  params->paramDefine(set, kOfxParamTypeDouble, "crash", &p);
  props->propSetInt(p, kOfxParamPropSecret, 0, 1);
  return kOfxStatOK;
}

static OfxStatus render(OfxImageEffectHandle e, OfxPropertySetHandle in) {
  double t = 0;
  props->propGetDouble(in, kOfxPropTime, 0, &t);
  OfxParamSetHandle set;
  fx->getParamSet(e, &set);
  OfxParamHandle gp, bp, cp;
  params->paramGetHandle(set, "gain", &gp, NULL);
  params->paramGetHandle(set, "bottom", &bp, NULL);
  params->paramGetHandle(set, "crash", &cp, NULL);
  double gain = 1, crash = 0;
  int bottom = 0;
  params->paramGetValueAtTime(gp, t, &gain);
  params->paramGetValueAtTime(bp, t, &bottom);
  params->paramGetValueAtTime(cp, t, &crash);
  if (crash > 0) abort();

  OfxImageClipHandle src_clip, dst_clip;
  fx->clipGetHandle(e, kOfxImageEffectSimpleSourceClipName, &src_clip, NULL);
  fx->clipGetHandle(e, kOfxImageEffectOutputClipName, &dst_clip, NULL);
  OfxPropertySetHandle src, dst;
  if (fx->clipGetImage(src_clip, t, NULL, &src) != kOfxStatOK) return kOfxStatFailed;
  if (fx->clipGetImage(dst_clip, t, NULL, &dst) != kOfxStatOK) {
    fx->clipReleaseImage(src);
    return kOfxStatFailed;
  }
  void *sp, *dp;
  int sb[4], db[4], srow, drow;
  props->propGetPointer(src, kOfxImagePropData, 0, &sp);
  props->propGetPointer(dst, kOfxImagePropData, 0, &dp);
  props->propGetIntN(src, kOfxImagePropBounds, 4, sb);
  props->propGetIntN(dst, kOfxImagePropBounds, 4, db);
  props->propGetInt(src, kOfxImagePropRowBytes, 0, &srow);
  props->propGetInt(dst, kOfxImagePropRowBytes, 0, &drow);
  int win[4];
  props->propGetIntN(in, kOfxImageEffectPropRenderWindow, 4, win);
  int h = db[3] - db[1];
  for (int y = win[1]; y < win[3]; y++) {
    float *s = (float *)((char *)sp + (size_t)(y - sb[1]) * (size_t)srow);
    float *d = (float *)((char *)dp + (size_t)(y - db[1]) * (size_t)drow);
    float g = (!bottom || y - db[1] < h / 2) ? (float)gain : 1.0f;
    for (int x = win[0]; x < win[2]; x++) {
      const float *a = s + 4 * (x - sb[0]);
      float *b = d + 4 * (x - db[0]);
      b[0] = a[0] * g;
      b[1] = a[1] * g;
      b[2] = a[2] * g;
      b[3] = a[3];
    }
  }
  fx->clipReleaseImage(src);
  fx->clipReleaseImage(dst);
  return kOfxStatOK;
}

static OfxStatus entry(const char *action, const void *handle, OfxPropertySetHandle in,
                       OfxPropertySetHandle out) {
  (void)out;
  OfxImageEffectHandle e = (OfxImageEffectHandle)handle;
  if (!strcmp(action, kOfxActionLoad)) {
    props = host->fetchSuite(host->host, kOfxPropertySuite, 1);
    fx = host->fetchSuite(host->host, kOfxImageEffectSuite, 1);
    params = host->fetchSuite(host->host, kOfxParameterSuite, 1);
    return props && fx && params ? kOfxStatOK : kOfxStatErrMissingHostFeature;
  }
  if (!strcmp(action, kOfxActionDescribe)) return describe(e);
  if (!strcmp(action, kOfxImageEffectActionDescribeInContext)) return describe_in_context(e);
  if (!strcmp(action, kOfxImageEffectActionRender)) return render(e, in);
  return kOfxStatReplyDefault;
}

static OfxPlugin plugin = {
    kOfxImageEffectPluginApi, 1, "dev.debut.test.Gain", 1, 0, set_host, entry,
};

OfxExport int OfxGetNumberOfPlugins(void) { return 1; }
OfxExport OfxPlugin *OfxGetPlugin(int i) { return i == 0 ? &plugin : NULL; }
