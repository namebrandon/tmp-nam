#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <limits>
#include <vector>

#include "NAM/activations.h"

namespace {

uint32_t bits(float value) {
  uint32_t result = 0;
  static_assert(sizeof(result) == sizeof(value), "float must be binary32");
  std::memcpy(&result, &value, sizeof(result));
  return result;
}

void scalar_reference(float* data, long size, float negative_slope) {
  // Volatile keeps this oracle scalar even in the performance build.
  for (long pos = 0; pos < size; ++pos) {
    volatile float input = data[pos];
    data[pos] = input > 0.0f ? input : negative_slope * input;
  }
}

void require_same(float expected, float actual) {
  if (std::isnan(expected))
    assert(std::isnan(actual));
  else
    assert(bits(expected) == bits(actual));
}

void test_boundaries_and_special_values() {
  const float denorm = std::numeric_limits<float>::denorm_min();
  const float normal = std::numeric_limits<float>::min();
  const float inf = std::numeric_limits<float>::infinity();
  const float nan = std::numeric_limits<float>::quiet_NaN();
  const std::vector<float> values = {
      -17.0f, -normal, -denorm, -0.0f, 0.0f, denorm, normal, 0.125f,
      1.0f, std::numeric_limits<float>::max(), -inf, inf, nan};
  const std::vector<long> lengths = {0, 1, 2, 3, 4, 5, 7, 8, 9,
                                     15, 16, 17, 31, 32, 33};
  const std::vector<float> slopes = {0.0f, 0.01f, 0.25f, 1.0f, -0.5f, 2.0f};

  for (float slope : slopes) {
    for (long length : lengths) {
      for (std::size_t offset = 0; offset < 4; ++offset) {
        std::vector<float> actual(static_cast<std::size_t>(length) + offset + 4, 1234.5f);
        for (long pos = 0; pos < length; ++pos)
          actual[offset + static_cast<std::size_t>(pos)] =
              values[static_cast<std::size_t>(pos) % values.size()];
        std::vector<float> expected = actual;
        scalar_reference(expected.data() + offset, length, slope);
        nam::activations::ActivationLeakyReLU activation(slope);
        activation.apply(actual.data() + offset, length);

        for (std::size_t pos = 0; pos < actual.size(); ++pos) {
          if (pos >= offset && pos < offset + static_cast<std::size_t>(length))
            require_same(expected[pos], actual[pos]);
          else
            assert(bits(expected[pos]) == bits(actual[pos]));
        }
      }
    }
  }
}

void test_long_unaligned_buffer() {
  constexpr long length = 1025;
  std::vector<float> actual(static_cast<std::size_t>(length) + 3);
  for (long pos = 0; pos < length; ++pos) {
    const float magnitude = static_cast<float>((pos % 97) + 1) / 101.0f;
    actual[1 + static_cast<std::size_t>(pos)] = pos % 3 == 0 ? -magnitude : magnitude;
  }
  std::vector<float> expected = actual;
  scalar_reference(expected.data() + 1, length, 0.0137f);
  nam::activations::ActivationLeakyReLU activation(0.0137f);
  activation.apply(actual.data() + 1, length);
  for (long pos = 0; pos < length; ++pos)
    require_same(expected[1 + static_cast<std::size_t>(pos)],
                 actual[1 + static_cast<std::size_t>(pos)]);
}

} // namespace

int main() {
  test_boundaries_and_special_values();
  test_long_unaligned_buffer();
  std::cout << "NAM LeakyReLU scalar/NEON parity: PASS\n";
}
