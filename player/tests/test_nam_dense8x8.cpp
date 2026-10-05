#include <array>
#include <atomic>
#include <cassert>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <new>
#include <string>
#include <vector>

#include <Eigen/Dense>

#include "NAM/conv1d.h"
#include "NAM/dsp.h"

namespace
{
std::atomic<std::size_t> gAllocations{0};
}

void* operator new(std::size_t size)
{
  if (void* ptr = std::malloc(size ? size : 1))
  {
    gAllocations.fetch_add(1, std::memory_order_relaxed);
    return ptr;
  }
  throw std::bad_alloc();
}

void* operator new[](std::size_t size) { return ::operator new(size); }
void operator delete(void* ptr) noexcept { std::free(ptr); }
void operator delete[](void* ptr) noexcept { std::free(ptr); }
void operator delete(void* ptr, std::size_t) noexcept { std::free(ptr); }
void operator delete[](void* ptr, std::size_t) noexcept { std::free(ptr); }

namespace
{
uint32_t Bits(const float value)
{
  uint32_t result;
  std::memcpy(&result, &value, sizeof(result));
  return result;
}

void RequireSame(const float expected, const float actual)
{
  if (std::isnan(expected))
    assert(std::isnan(actual));
  else
    assert(Bits(expected) == Bits(actual));
}

void RequireSame(const Eigen::MatrixXf& expected, const Eigen::MatrixXf& actual, const int frames)
{
  assert(expected.rows() == actual.rows());
  assert(frames <= expected.cols() && frames <= actual.cols());
  for (int frame = 0; frame < frames; ++frame)
    for (int row = 0; row < expected.rows(); ++row)
      RequireSame(expected(row, frame), actual(row, frame));
}

float WeightValue(const int tap, const int output, const int input)
{
  const int raw = ((tap + 3) * 97 + (output + 5) * 37 + (input + 7) * 19) % 61 - 30;
  return static_cast<float>(raw) / 29.0f;
}

float InputValue(const long frame, const int channel)
{
  const float value = 0.41f * std::sin(static_cast<float>(frame * 8 + channel) * 0.071f)
    + 0.17f * std::cos(static_cast<float>(frame * 3 - channel) * 0.113f);
  if ((frame + channel * 11) % 101 == 0)
    return channel & 1 ? -0.0f : 0.0f;
  if ((frame + channel * 17) % 137 == 0)
    return channel & 1 ? -std::numeric_limits<float>::denorm_min() : std::numeric_limits<float>::denorm_min();
  return value;
}

Eigen::MatrixXf DenseProduct(const Eigen::MatrixXf& weights, const Eigen::MatrixXf& input, const int frames,
                             const Eigen::MatrixXf* destination = nullptr)
{
  Eigen::MatrixXf result(8, frames);
  for (int frame = 0; frame < frames; ++frame)
  {
    for (int output = 0; output < 8; ++output)
    {
      float product = 0.0f;
      for (int input_channel = 0; input_channel < 8; ++input_channel)
        product = std::fma(weights(output, input_channel), input(input_channel, frame), product);
      const float previous = destination == nullptr ? 0.0f : (*destination)(output, frame);
      result(output, frame) = std::fma(product, 1.0f, previous);
    }
  }
  return result;
}

void AppendMatrix(std::vector<float>& output, const Eigen::MatrixXf& matrix, const int frames)
{
  output.insert(output.end(), matrix.data(), matrix.data() + matrix.rows() * frames);
}

#if defined(__aarch64__) && defined(NAM_DENSE8X8_INTEGRATION_CANDIDATE)
uint64_t ReadFpcr()
{
  uint64_t result;
  __asm__ volatile("mrs %0, fpcr" : "=r"(result));
  return result;
}

void WriteFpcr(const uint64_t value) { __asm__ volatile("msr fpcr, %0" : : "r"(value)); }

float PatternValue(const int pattern, const std::size_t index)
{
  switch (pattern)
  {
  case 0:
  {
    uint32_t state = static_cast<uint32_t>(index + 1) * 747796405u + 2891336453u;
    state = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    state = (state >> 22u) ^ state;
    return static_cast<float>(static_cast<int32_t>(state)) / 2147483648.0f;
  }
  case 1:
    return (index & 1) ? -1048576.0f : 1048576.0f;
  case 2:
    return (index & 1) ? -0.0f : 0.0f;
  case 3:
    return (index & 1) ? -std::numeric_limits<float>::denorm_min()
                       : std::numeric_limits<float>::denorm_min();
  case 4:
    return (index % 3 == 0) ? std::numeric_limits<float>::max() / 16.0f
                            : ((index & 1) ? -0x1p-60f : 0x1p-60f);
  default:
    switch (index % 7)
    {
    case 0:
      return std::numeric_limits<float>::infinity();
    case 1:
      return -std::numeric_limits<float>::infinity();
    case 2:
      return std::numeric_limits<float>::quiet_NaN();
    case 3:
      return -0.0f;
    default:
      return static_cast<float>(static_cast<int>(index % 19) - 9) / 11.0f;
    }
  }
}

void TestHelper()
{
  constexpr uint64_t kFz = uint64_t{1} << 24;
  constexpr uint64_t kDn = uint64_t{1} << 25;
  const uint64_t saved_fpcr = ReadFpcr();
  const std::array<uint64_t, 3> modes = {saved_fpcr & ~(kFz | kDn), (saved_fpcr & ~(kFz | kDn)) | kFz,
                                         (saved_fpcr & ~(kFz | kDn)) | kDn};
  const std::array<int, 3> frames = {32, 34, 35};
  const std::array<int, 3> strides = {8, 9, 13};
  for (const uint64_t mode : modes)
  {
    WriteFpcr(mode);
    for (const int count : frames)
      for (const int stride : strides)
        for (int pattern = 0; pattern < 6; ++pattern)
          for (int offset = 0; offset < 4; ++offset)
            for (const bool assign : {false, true})
            {
              std::vector<float> weight_storage(64 + offset + 4, 9123.0f);
              std::vector<float> input_storage(static_cast<std::size_t>(stride * count + offset + 4), 8123.0f);
              std::vector<float> output_storage(static_cast<std::size_t>(8 * count + offset + 4), 7123.0f);
              float* const weights = weight_storage.data() + offset;
              float* const input = input_storage.data() + offset;
              float* const output = output_storage.data() + offset;
              for (int index = 0; index < 64; ++index)
                weights[index] = PatternValue(pattern, static_cast<std::size_t>(index * 5 + 1));
              for (int frame = 0; frame < count; ++frame)
                for (int channel = 0; channel < 8; ++channel)
                  input[frame * stride + channel] = PatternValue(pattern, static_cast<std::size_t>(frame * 13 + channel));
              for (int index = 0; index < 8 * count; ++index)
                output[index] = assign ? PatternValue(0, static_cast<std::size_t>(index + 991))
                                       : PatternValue(pattern, static_cast<std::size_t>(index + 313));
              std::vector<float> expected = output_storage;
              for (int frame = 0; frame < count; ++frame)
                for (int row = 0; row < 8; ++row)
                {
                  float product = 0.0f;
                  for (int depth = 0; depth < 8; ++depth)
                    product = std::fma(weights[depth * 8 + row], input[frame * stride + depth], product);
                  const float previous = assign ? 0.0f : expected[offset + frame * 8 + row];
                  expected[offset + frame * 8 + row] = std::fma(product, 1.0f, previous);
                }
              gAllocations.store(0, std::memory_order_relaxed);
              assert(nam::detail::Dense8x8Product(weights, input, count, stride, 1, output, assign));
              assert(gAllocations.load(std::memory_order_relaxed) == 0);
              for (std::size_t index = 0; index < expected.size(); ++index)
                RequireSame(expected[index], output_storage[index]);
            }
  }
  WriteFpcr(saved_fpcr);

  for (const int count : {0, 1, 31, 33, 36, 64, 127, 256})
  {
    std::vector<float> input(static_cast<std::size_t>(8 * count + 8), 1.0f);
    std::vector<float> output(static_cast<std::size_t>(8 * count + 8), -3.0f);
    const std::vector<float> before = output;
    std::array<float, 64> weights{};
    assert(!nam::detail::Dense8x8Product(weights.data(), input.data(), count, 8, 1, output.data(), true));
    assert(output == before);
  }
  std::array<float, 64> weights{};
  std::array<float, 512> input{};
  std::array<float, 256> output{};
  assert(!nam::detail::Dense8x8Product(weights.data(), input.data(), 32, 16, 2, output.data(), true));
}
#endif

std::vector<Eigen::MatrixXf> MakeTapWeights(const int taps)
{
  std::vector<Eigen::MatrixXf> weights;
  weights.reserve(static_cast<std::size_t>(taps));
  for (int tap = 0; tap < taps; ++tap)
  {
    Eigen::MatrixXf matrix(8, 8);
    for (int output = 0; output < 8; ++output)
      for (int input = 0; input < 8; ++input)
        matrix(output, input) = WeightValue(tap, output, input);
    weights.push_back(std::move(matrix));
  }
  return weights;
}

std::vector<float> SerializeConv1D(const std::vector<Eigen::MatrixXf>& weights, const Eigen::VectorXf& bias)
{
  std::vector<float> serialized;
  serialized.reserve(weights.size() * 64 + static_cast<std::size_t>(bias.size()));
  for (int output = 0; output < 8; ++output)
    for (int input = 0; input < 8; ++input)
      for (const auto& tap : weights)
        serialized.push_back(tap(output, input));
  for (int output = 0; output < bias.size(); ++output)
    serialized.push_back(bias(output));
  return serialized;
}

void TestConv1x1(std::vector<float>& transcript)
{
  Eigen::MatrixXf weights(8, 8);
  Eigen::VectorXf bias(8);
  std::vector<float> serialized;
  serialized.reserve(72);
  for (int output = 0; output < 8; ++output)
    for (int input = 0; input < 8; ++input)
    {
      weights(output, input) = WeightValue(0, output, input);
      serialized.push_back(weights(output, input));
    }
  for (int output = 0; output < 8; ++output)
  {
    bias(output) = static_cast<float>(output - 3) / 17.0f;
    serialized.push_back(bias(output));
  }

  nam::Conv1x1 layer(8, 8, true, 1);
  auto iterator = serialized.begin();
  layer.set_weights_(iterator);
  assert(iterator == serialized.end());
  layer.SetMaxBufferSize(256);
  Eigen::MatrixXf storage(13, 256);
  for (int frame = 0; frame < storage.cols(); ++frame)
    for (int row = 0; row < storage.rows(); ++row)
      storage(row, frame) = InputValue(frame, row);

  for (const int count : {0, 1, 31, 32, 33, 34, 35, 36, 64, 127, 256})
  {
    const auto input = storage.topRows(8);
    Eigen::MatrixXf expected;
    if (count == 32 || count == 34 || count == 35)
    {
      expected = DenseProduct(weights, input, count);
      expected.colwise() += bias;
    }
    layer.process_(input, count);
    if (count == 32 || count == 34 || count == 35)
      RequireSame(expected, layer.GetOutput(), count);
    AppendMatrix(transcript, layer.GetOutput(), count);

    if (count == 34)
    {
      Eigen::MatrixXf contiguous = input.leftCols(count);
      const Eigen::MatrixXf returned = layer.process(contiguous, count);
      const Eigen::MatrixXf returned_expected = DenseProduct(weights, contiguous, count);
      Eigen::MatrixXf biased = returned_expected;
      biased.colwise() += bias;
      RequireSame(biased, returned, count);
    }
  }

  Eigen::MatrixXf no_alloc_input = storage.topRows(8).leftCols(35);
  gAllocations.store(0, std::memory_order_relaxed);
  layer.process_(no_alloc_input, 35);
  assert(gAllocations.load(std::memory_order_relaxed) == 0);
}

void TestConv1D(const int taps, const int dilation, const bool with_bias, std::vector<float>& transcript)
{
  const auto weights = MakeTapWeights(taps);
  Eigen::VectorXf bias(with_bias ? 8 : 0);
  for (int output = 0; output < bias.size(); ++output)
    bias(output) = static_cast<float>(output - 5) / 31.0f;
  std::vector<float> serialized = SerializeConv1D(weights, bias);
  nam::Conv1D layer(8, 8, taps, with_bias ? 1 : 0, dilation, 1);
  auto iterator = serialized.begin();
  layer.set_weights_(iterator);
  assert(iterator == serialized.end());
  layer.SetMaxBufferSize(256);

  std::vector<std::array<float, 8>> history;
  long global_frame = 0;
  const std::array<int, 22> sequence = {0, 1, 31, 32, 33, 34, 35, 36, 64, 127, 256,
                                         35, 34, 32, 1, 256, 127, 36, 35, 34, 33, 32};
  for (const int count : sequence)
  {
    Eigen::MatrixXf input(8, 256);
    for (int frame = 0; frame < input.cols(); ++frame)
      for (int channel = 0; channel < 8; ++channel)
        input(channel, frame) = InputValue(global_frame + frame, channel);
    const std::size_t before = history.size();
    for (int frame = 0; frame < count; ++frame)
    {
      std::array<float, 8> sample{};
      for (int channel = 0; channel < 8; ++channel)
        sample[static_cast<std::size_t>(channel)] = input(channel, frame);
      history.push_back(sample);
    }

    Eigen::MatrixXf expected(8, count);
    if (count == 32 || count == 34 || count == 35)
    {
      expected.setZero();
      for (int tap = 0; tap < taps; ++tap)
      {
        Eigen::MatrixXf tap_input(8, count);
        const long lookback = static_cast<long>(dilation) * (taps - 1 - tap);
        for (int frame = 0; frame < count; ++frame)
        {
          const long source = static_cast<long>(before) + frame - lookback;
          for (int channel = 0; channel < 8; ++channel)
            tap_input(channel, frame) = source < 0 ? 0.0f : history[static_cast<std::size_t>(source)][channel];
        }
        expected = DenseProduct(weights[static_cast<std::size_t>(tap)], tap_input, count, &expected);
      }
      if (with_bias)
        expected.colwise() += bias;
    }

    layer.Process(input, count);
    if (count == 32 || count == 34 || count == 35)
      RequireSame(expected, layer.GetOutput(), count);
    AppendMatrix(transcript, layer.GetOutput(), count);
    global_frame += count;
  }

  nam::Conv1D no_alloc_layer(8, 8, taps, with_bias ? 1 : 0, dilation, 1);
  iterator = serialized.begin();
  no_alloc_layer.set_weights_(iterator);
  no_alloc_layer.SetMaxBufferSize(256);
  Eigen::MatrixXf no_alloc_input(8, 35);
  for (int frame = 0; frame < 35; ++frame)
    for (int channel = 0; channel < 8; ++channel)
      no_alloc_input(channel, frame) = InputValue(frame, channel);
  gAllocations.store(0, std::memory_order_relaxed);
  no_alloc_layer.Process(no_alloc_input, 35);
  assert(gAllocations.load(std::memory_order_relaxed) == 0);
}

} // namespace

int main(int argc, char** argv)
{
#if defined(__aarch64__) && defined(NAM_DENSE8X8_INTEGRATION_CANDIDATE)
  TestHelper();
#endif
  std::vector<float> transcript;
  TestConv1x1(transcript);
  for (const int taps : {3, 6, 15})
    for (const int dilation : {1, 2, 7})
      for (const bool bias : {false, true})
        TestConv1D(taps, dilation, bias, transcript);
#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
  const auto stats = nam::detail::GetDense8x8Diagnostics();
  assert(stats.conv1x1_calls > 0 && stats.conv1x1_frames > 0);
  assert(stats.conv1d_calls > 0 && stats.conv1d_frames > 0 && stats.conv1d_tap_products > 0);
#endif
  if (argc == 2)
  {
    std::ofstream output(argv[1], std::ios::binary);
    assert(output);
    output.write(reinterpret_cast<const char*>(transcript.data()),
                 static_cast<std::streamsize>(transcript.size() * sizeof(float)));
    assert(output);
  }
  std::cout << "NAM dense 8x8 helper/integration exactness: PASS (" << transcript.size() << " values)\n";
}
