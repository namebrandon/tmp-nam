// RingBuffer: every Read() must return exactly the frames a plain history would,
// in both modes (rewind for short lookbacks, mirrored ring for long ones), for
// random block sizes up to max_buffer_size.
#include <cassert>
#include <iostream>
#include <random>
#include <vector>

#include "NAM/ring_buffer.h"

int main() {
  const int kChannels = 3, kMaxBuffer = 343;
  std::mt19937 rng(11);
  std::uniform_int_distribution<int> block(1, kMaxBuffer);
  std::normal_distribution<float> value(0.0f, 1.0f);
  for (const long lookback : {0L, 2L, 256L, 342L, 343L, 344L, 1024L, 2046L}) {
    nam::RingBuffer rb;
    rb.SetMaxLookback(lookback);
    rb.Reset(kChannels, kMaxBuffer);
    std::vector<std::vector<float>> history;  // every frame ever written
    Eigen::MatrixXf input(kChannels, kMaxBuffer);
    for (int iter = 0; iter < 4000; iter++) {
      const int n = block(rng);
      for (int f = 0; f < n; f++)
        for (int c = 0; c < kChannels; c++)
          input(c, f) = value(rng);
      rb.Write(input, n);
      const long first = static_cast<long>(history.size());
      for (int f = 0; f < n; f++) {
        history.emplace_back(kChannels);
        for (int c = 0; c < kChannels; c++) history.back()[static_cast<size_t>(c)] = input(c, f);
      }
      for (const long l : {0L, lookback / 2, lookback}) {
        auto got = rb.Read(n, l);
        for (int f = 0; f < n; f++) {
          const long frame = first - l + f;
          for (int c = 0; c < kChannels; c++) {
            const float want = frame < 0 ? 0.0f : history[static_cast<size_t>(frame)][static_cast<size_t>(c)];
            assert(got(c, f) == want);
          }
        }
      }
      rb.Advance(n);
    }
  }
  std::cout << "NAM ring buffer: PASS\n";
  return 0;
}
