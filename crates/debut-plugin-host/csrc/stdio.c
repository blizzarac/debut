/* Keep the protocol pipe clean: plugins that print to stdout end up on stderr. */
#include <unistd.h>

int debut_protect_stdout(void) {
  int fd = dup(1);
  if (fd < 0) return -1;
  if (dup2(2, 1) < 0) return -1;
  return fd;
}
