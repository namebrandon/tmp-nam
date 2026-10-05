/* Old glibc headers define major/minor macros that collide with Core members. */
#include <sys/types.h>
#ifdef major
#undef major
#endif
#ifdef minor
#undef minor
#endif
