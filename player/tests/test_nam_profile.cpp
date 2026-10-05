#include "../nam_profile.h"

#include <cassert>
#include <iostream>

int main() {
  tmp_nam::UtilizationHistogram histogram;
  auto empty = histogram.snapshot();
  assert(empty.samples == 0 && empty.overflow == 0);

  // 32 frames at 44.1 kHz have a 725.6 us period. The nearest-rank p99.9
  // below consists of 998 ordinary callbacks, one 74.5% callback, and one
  // hard miss; the p99.9 is therefore the 74.5% observation.
  for (int i = 0; i < 998; ++i) histogram.observe(360000, 32, 44100);
  histogram.observe(540000, 32, 44100);
  histogram.observe(800000, 32, 44100);
  auto snapshot = histogram.snapshot();
  assert(snapshot.samples == 1000);
  assert(snapshot.overflow == 0);
  assert(snapshot.p999_permille_upper == 745);
  assert(!snapshot.p999_overflow);

  tmp_nam::UtilizationHistogram overflow;
  for (int i = 0; i < 1000; ++i) overflow.observe(2000000, 32, 44100);
  snapshot = overflow.snapshot();
  assert(snapshot.samples == 1000);
  assert(snapshot.overflow == 1000);
  assert(snapshot.p999_overflow);
  assert(snapshot.p999_permille_upper ==
         tmp_nam::UtilizationHistogram::kMaxPermille + 1);

  std::cout << "NAM fixed callback-utilization histogram: PASS\n";
}
