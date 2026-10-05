#include "../nam_player.h"

#include <algorithm>
#include <atomic>
#include <cassert>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <set>
#include <iostream>
#include <memory>
#include <new>
#include <stdexcept>
#include <vector>

#include "NAM/get_dsp.h"
#include "NAM/container.h"
#include "NAM/slimmable.h"

namespace {

constexpr float kPi = 3.14159265358979323846f;
std::atomic<std::size_t> g_allocations{0};
}

void* operator new(std::size_t size) {
  if (void* ptr = std::malloc(size ? size : 1)) {
    g_allocations.fetch_add(1, std::memory_order_relaxed);
    return ptr;
  }
  throw std::bad_alloc();
}

void* operator new[](std::size_t size) {
  return ::operator new(size);
}

void operator delete(void* ptr) noexcept { std::free(ptr); }
void operator delete[](void* ptr) noexcept { std::free(ptr); }
void operator delete(void* ptr, std::size_t) noexcept { std::free(ptr); }
void operator delete[](void* ptr, std::size_t) noexcept { std::free(ptr); }

namespace {

class LinearDSP final : public nam::DSP {
 public:
  LinearDSP(double rate, float gain, int input_channels = 1, int output_channels = 1)
      : nam::DSP(input_channels, output_channels, rate), gain_(gain) {}

  void process(NAM_SAMPLE** input, NAM_SAMPLE** output, int frames) override {
    for (int i = 0; i < frames; ++i) output[0][i] = gain_ * input[0][i];
  }

 private:
  float gain_;
};

class SlimmableDSP final : public nam::DSP, public nam::SlimmableModel {
 public:
  explicit SlimmableDSP(double rate) : nam::DSP(1, 1, rate) {}

  void SetSlimmableSize(double value) override {
    selected_size = value;
    gain = static_cast<float>(1.0 + value);
  }

  void process(NAM_SAMPLE** input, NAM_SAMPLE** output, int frames) override {
    for (int i = 0; i < frames; ++i) output[0][i] = gain * input[0][i];
  }

  double selected_size = -1.0;
  float gain = 1.0f;
};

class PlayerCountingDSP final : public nam::DSP {
 public:
  explicit PlayerCountingDSP(float value) : nam::DSP(1, 1, 48000.0), value_(value) {}

  void Reset(double sample_rate, int max_buffer_size) override {
    ++reset_calls;
    last_sample_rate = sample_rate;
    last_buffer_size = max_buffer_size;
    nam::DSP::Reset(sample_rate, max_buffer_size);
  }

  void prewarm() override { ++prewarm_calls; }

  void process(NAM_SAMPLE**, NAM_SAMPLE** output, int frames) override {
    for (int i = 0; i < frames; ++i) output[0][i] = value_;
  }

  int reset_calls = 0;
  int prewarm_calls = 0;
  double last_sample_rate = 0.0;
  int last_buffer_size = -1;

 private:
  float value_;
};

std::vector<float> input_signal(std::size_t n) {
  std::vector<float> result(n);
  for (std::size_t i = 0; i < n; ++i)
    result[i] = 0.3f * std::sin(static_cast<float>(i) * 0.071f) +
                0.1f * std::cos(static_cast<float>(i) * 0.013f);
  return result;
}

void test_equal_rate_irregular_and_in_place() {
  constexpr std::size_t frames = 1031;
  const auto input = input_signal(frames);
  auto separate_dsp = std::make_unique<LinearDSP>(44100.0, 2.0f);
  auto in_place_dsp = std::make_unique<LinearDSP>(44100.0, 2.0f);
  tmp_nam::Player separate(std::move(separate_dsp));
  tmp_nam::Player in_place(std::move(in_place_dsp));

  std::vector<float> expected(frames, -99.0f);
  std::vector<float> actual = input;
  std::size_t pos = 0;
  const std::size_t chunks[] = {1, 17, 256, 3, 129, 625};
  for (std::size_t chunk : chunks) {
    const std::size_t n = std::min(chunk, frames - pos);
    separate.process(input.data() + pos, expected.data() + pos, n);
    in_place.process(actual.data() + pos, actual.data() + pos, n);
    pos += n;
    if (pos == frames) break;
  }
  assert(pos == frames);
  for (std::size_t i = 0; i < frames; ++i) assert(actual[i] == expected[i]);
  assert(separate.model_rate() == 44100);
  assert(separate.latency_frames() == 0);
  assert(separate.engine_frames() == frames);
  assert(separate.model_frames() == frames);
  assert(separate.process_calls() == 6);
}

void test_rate_conversion_has_no_block_drift() {
  constexpr std::size_t frames = 44100;
  const auto input = input_signal(frames);
  for (int engine_rate : {44100, 48000}) {
    const int model_rate = engine_rate == 44100 ? 48000 : 44100;
    tmp_nam::Player::Options options;
    options.engine_rate = engine_rate;
    options.max_block = 256;
    options.resampler_quality = 5;
    tmp_nam::Player player(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
    std::vector<float> output(frames, -777.0f);

    std::size_t pos = 0;
    const std::size_t chunks[] = {7, 31, 256, 2, 89, 511, 13};
    std::size_t chunk_index = 0;
    while (pos < frames) {
      const std::size_t requested = chunks[chunk_index++ % (sizeof(chunks) / sizeof(chunks[0]))];
      const std::size_t n = std::min(requested, frames - pos);
      player.process(input.data() + pos, output.data() + pos, n);
      pos += n;
    }

    assert(player.model_rate() == model_rate);
    assert(player.engine_frames() == frames);
    // Stateful resampling must track the ratio continuously; an independent
    // round() per callback would accumulate a large error here.
    const double expected = static_cast<double>(frames) * model_rate / engine_rate;
    assert(std::abs(static_cast<double>(player.model_frames()) - expected) <= 2.0);
    assert(std::isfinite(player.latency_seconds()));
    assert(player.latency_seconds() <= 0.003);
    assert(player.underflow_frames() == 0);
    assert(player.late_underflow_frames() == 0);
    for (float value : output) assert(std::isfinite(value));
  }
}

void test_rate_conversion_is_chunking_invariant() {
  constexpr std::size_t frames = 5000;
  const auto input = input_signal(frames);
  for (int engine_rate : {44100, 48000}) {
    const int model_rate = engine_rate == 44100 ? 48000 : 44100;
    for (int max_block : {32, 256}) {
      tmp_nam::Player::Options options;
      options.engine_rate = engine_rate;
      options.max_block = max_block;
      options.resampler_quality = 6;
      tmp_nam::Player one_call(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
      tmp_nam::Player many_calls(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
      std::vector<float> one(frames, 0.0f), many(frames, 0.0f);
      one_call.process(input.data(), one.data(), frames);

      std::size_t pos = 0;
      while (pos < frames) {
        const std::size_t n = std::min<std::size_t>((pos % 251) + 1, frames - pos);
        many_calls.process(input.data() + pos, many.data() + pos, n);
        pos += n;
      }
      for (std::size_t i = 0; i < frames; ++i)
        assert(std::abs(one[i] - many[i]) < 1.0e-5f);
      assert(one_call.engine_frames() == many_calls.engine_frames());
      assert(one_call.model_frames() == many_calls.model_frames());
      assert(one_call.late_underflow_frames() == 0);
      assert(many_calls.late_underflow_frames() == 0);
    }
  }
}

void test_single_frame_calls_and_no_allocations() {
  constexpr std::size_t frames = 2048;
  const auto input = input_signal(frames);
  for (int engine_rate : {44100, 48000}) {
    const int model_rate = engine_rate == 44100 ? 48000 : 44100;
    tmp_nam::Player::Options options;
    options.engine_rate = engine_rate;
    options.max_block = 32;
    options.resampler_quality = 5;
    tmp_nam::Player one_call(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
    tmp_nam::Player single_calls(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
    std::vector<float> one(frames, 0.0f), many(frames, 0.0f);
    one_call.process(input.data(), one.data(), frames);
    g_allocations.store(0, std::memory_order_relaxed);
    for (std::size_t i = 0; i < frames; ++i)
      single_calls.process(input.data() + i, many.data() + i, 1);
    assert(g_allocations.load(std::memory_order_relaxed) == 0);
    for (std::size_t i = 0; i < frames; ++i)
      assert(std::abs(one[i] - many[i]) < 1.0e-5f);
    assert(single_calls.underflow_frames() == 0);
    assert(single_calls.late_underflow_frames() == 0);
  }
}

void test_impulse_latency_and_conversion_sweep() {
  for (int quality : {5, 6}) {
    for (int engine_rate : {44100, 48000}) {
      const int model_rate = engine_rate == 44100 ? 48000 : 44100;
      tmp_nam::Player::Options options;
      options.engine_rate = engine_rate;
      options.max_block = 256;
      options.resampler_quality = quality;
      tmp_nam::Player impulse_player(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
      constexpr std::size_t frames = 4096;
      std::vector<float> impulse(frames, 0.0f), output(frames, 0.0f);
      impulse[0] = 1.0f;
      impulse_player.process(impulse.data(), output.data(), frames);
      std::size_t peak = 0;
      for (std::size_t i = 1; i < frames; ++i)
        if (std::abs(output[i]) > std::abs(output[peak])) peak = i;
      const std::size_t latency = impulse_player.latency_frames();
      const std::size_t distance = peak > latency ? peak - latency : latency - peak;
      std::cout << "NAM player q" << quality << " " << engine_rate << "->" << model_rate
                << " latency=" << latency << " frames ("
                << impulse_player.latency_seconds() * 1000.0
                << " ms), impulse_peak=" << peak << "\n";
      assert(distance <= 1);
      assert(impulse_player.late_underflow_frames() == 0);

      for (float frequency : {20.0f, 1000.0f, 5000.0f, 10000.0f, 18000.0f}) {
        tmp_nam::Player sweep_player(std::make_unique<LinearDSP>(model_rate, 1.0f), options);
        std::vector<float> input(frames), sweep_output(frames, 0.0f);
        for (std::size_t i = 0; i < frames; ++i)
          input[i] = std::sin(2.0f * kPi * frequency *
                              static_cast<float>(i) / static_cast<float>(engine_rate));
        sweep_player.process(input.data(), sweep_output.data(), frames);
        double in_energy = 0.0;
        double out_energy = 0.0;
        const std::size_t begin = 512;
        const std::size_t end = frames - 512;
        for (std::size_t i = begin; i < end; ++i) {
          // Compare the same part of the sine after integer-delay alignment;
          // an unaligned short 20 Hz window measures phase, not filter gain.
          const auto aligned = i - sweep_player.latency_frames();
          in_energy += input[aligned] * input[aligned];
          out_energy += sweep_output[i] * sweep_output[i];
          assert(std::isfinite(sweep_output[i]));
        }
        const double ratio = std::sqrt(out_energy / in_energy);
        std::cout << "NAM player q" << quality << " " << engine_rate << "->" << model_rate
                  << " " << frequency << " Hz gain=" << ratio << "\n";
        assert(ratio > 0.99 && ratio < 1.01);
        assert(sweep_player.late_underflow_frames() == 0);
      }
    }
  }
}

void test_rate_and_mono_validation() {
  bool unknown_rate_rejected = false;
  try {
    tmp_nam::Player player(std::make_unique<LinearDSP>(96000.0, 1.0f));
  } catch (const std::invalid_argument&) {
    unknown_rate_rejected = true;
  }
  assert(unknown_rate_rejected);

  tmp_nam::Player::Options override_options;
  override_options.model_rate_override = 48000;
  tmp_nam::Player override_player(std::make_unique<LinearDSP>(0.0, 1.0f), override_options);
  assert(override_player.model_rate() == 48000);

  bool conflicting_override_rejected = false;
  try {
    tmp_nam::Player::Options conflict;
    conflict.model_rate_override = 48000;
    tmp_nam::Player player(std::make_unique<LinearDSP>(44100.0, 1.0f), conflict);
  } catch (const std::invalid_argument&) {
    conflicting_override_rejected = true;
  }
  assert(conflicting_override_rejected);

  bool stereo_rejected = false;
  try {
    tmp_nam::Player player(std::make_unique<LinearDSP>(44100.0, 1.0f, 2, 1));
  } catch (const std::invalid_argument&) {
    stereo_rejected = true;
  }
  assert(stereo_rejected);
}

void test_nested_stereo_container_rejected() {
  bool rejected = false;
  try {
    std::vector<nam::container::Submodel> children;
    children.push_back({1.0, std::make_unique<LinearDSP>(44100.0, 1.0f, 2, 1)});
    auto container = std::make_unique<nam::container::ContainerModel>(
        std::move(children), 44100.0);
    tmp_nam::Player player(std::move(container));
  } catch (const std::exception&) {
    rejected = true;
  }
  assert(rejected);
}

void test_slimmable_size_is_applied_during_prepare() {
  auto dsp = std::make_unique<SlimmableDSP>(44100.0);
  SlimmableDSP* raw = dsp.get();
  tmp_nam::Player::Options options;
  options.size = 0.25;
  tmp_nam::Player player(std::move(dsp), options);
  assert(raw->selected_size == 0.25);
  float input = 2.0f;
  float output = 0.0f;
  player.process(&input, &output, 1);
  assert(output == 2.5f);
}

void test_full_size_default() {
  auto dsp = std::make_unique<SlimmableDSP>(44100.0);
  auto* raw = dsp.get();
  tmp_nam::Player player(std::move(dsp));
  assert(raw->selected_size == 1.0);
}

void test_container_threshold_is_exclusive() {
  // A size of 0.5 is a selector, not a promise of half the channels. With
  // upstream thresholds [0.5, 1.0], it selects the second (full) submodel.
  for (double size : {0.0, std::nextafter(0.5, 0.0), 0.5, 1.0}) {
    std::vector<nam::container::Submodel> children;
    children.push_back({0.5, std::make_unique<LinearDSP>(44100.0, 2.0f)});
    children.push_back({1.0, std::make_unique<LinearDSP>(44100.0, 3.0f)});
    auto dsp = std::make_unique<nam::container::ContainerModel>(
        std::move(children), 44100.0);
    tmp_nam::Options options;
    options.size = size;
    tmp_nam::Player player(std::move(dsp), options);
    float input = 1.0f, output = 0.0f;
    player.process(&input, &output, 1);
    assert(output == (size < 0.5 ? 2.0f : 3.0f));
  }
}

void test_player_container_warms_only_final_selection() {
  for (double size : {0.0, 1.0}) {
    auto small = std::make_unique<PlayerCountingDSP>(1.0f);
    auto large = std::make_unique<PlayerCountingDSP>(2.0f);
    auto* small_raw = small.get();
    auto* large_raw = large.get();
    std::vector<nam::container::Submodel> children;
    children.push_back({0.5, std::move(small)});
    children.push_back({1.0, std::move(large)});
    auto container = std::make_unique<nam::container::ContainerModel>(
        std::move(children), 48000.0);

    tmp_nam::Player::Options options;
    options.engine_rate = 48000;
    options.max_block = 32;
    options.size = size;
    tmp_nam::Player player(std::move(container), options);

    auto* selected = size == 0.0 ? small_raw : large_raw;
    auto* unselected = size == 0.0 ? large_raw : small_raw;
    assert(selected->prewarm_calls == 1);
    assert(unselected->prewarm_calls == 0);
    assert(selected->last_sample_rate == 48000.0);
    assert(selected->last_buffer_size == 32);
    assert(!selected->GetPrewarmOnReset());
    assert(!unselected->GetPrewarmOnReset());

    float input = 0.0f;
    float output = 0.0f;
    player.process(&input, &output, 1);
    assert(output == (size == 0.0 ? 1.0f : 2.0f));
  }
}

void test_callback_exception_preserves_output() {
  class ThrowingDSP : public nam::DSP {
   public:
    ThrowingDSP() : nam::DSP(1, 1, 44100) {}
    bool fail = false;
    void process(NAM_SAMPLE** input, NAM_SAMPLE** output, int n) override {
      for (int i = 0; i < n; ++i) output[0][i] = input[0][i] * 2;
      if (fail && n) throw std::runtime_error("test DSP failure after writing");
    }
  };
  for (bool inplace : {false, true}) {
    auto dsp = std::make_unique<ThrowingDSP>();
    auto* raw = dsp.get();
    tmp_nam::Player player(std::move(dsp));
    raw->fail = true;
    auto input = input_signal(32);
    std::vector<float> output(32, 123);
    float* destination = inplace ? input.data() : output.data();
    const std::vector<float> before(destination, destination + 32);
    bool threw = false;
    try { player.process(input.data(), destination, 32); }
    catch (const std::runtime_error&) { threw = true; }
    assert(threw && std::equal(before.begin(), before.end(), destination));
  }
}

void test_real_upstream_slimmable_model_when_available() {
  namespace fs = std::filesystem;
  std::vector<fs::path> models;
  if (const char* env = std::getenv("TMP_NAM_SLIMMABLE_MODEL"); env && env[0]) {
    models.emplace_back(env);
  } else {
    const char* vendor = std::getenv("TMP_NAM_VENDOR_DIR");
    const fs::path examples = fs::path(vendor ? vendor : "player/stubs/vendor") /
        "NeuralAmpModelerCore/example_models";
    models.emplace_back(examples / "slimmable_container.nam");
    models.emplace_back(examples / "slimmable_wavenet.nam");
  }
  for (const fs::path& model : models) {
    if (!fs::exists(model)) throw std::runtime_error("required slimmable fixture missing: " + model.string());
    // Test each selectable network and both sides of its actual boundary.
    std::ifstream stream(model);
    const auto data = nlohmann::json::parse(stream);
    std::set<double> sizes{0.0, 0.5, 1.0};
    auto boundary = [&](double value) {
      if (value > 0 && value < 1) {
        sizes.insert(std::nextafter(value, 0.0));
        sizes.insert(value);
        sizes.insert(std::nextafter(value, 1.0));
      }
    };
    if (data.at("architecture") == "SlimmableContainer") {
      for (const auto& child : data.at("config").at("submodels"))
        boundary(child.at("max_value").get<double>());
    } else {
      for (const auto& layer : data.at("config").at("layers")) {
        if (!layer.contains("slimmable")) continue;
        const size_t count = layer.at("slimmable").at("kwargs").at("allowed_channels").size();
        for (size_t i = 1; i < count; ++i) boundary(static_cast<double>(i) / count);
      }
    }
    for (double size : sizes) {
      auto player_dsp = nam::get_dsp(model);
      auto* player_slimmable = dynamic_cast<nam::SlimmableModel*>(player_dsp.get());
      assert(player_slimmable != nullptr);
      const int sample_rate = static_cast<int>(player_dsp->GetExpectedSampleRate());
      assert(sample_rate == 44100 || sample_rate == 48000);
      tmp_nam::Player::Options options;
      options.engine_rate = sample_rate;
      options.max_block = 32;
      options.size = size;
      tmp_nam::Player player(std::move(player_dsp), options);

      const auto input = input_signal(256);
      std::vector<float> player_output(input.size(), 0.0f);
      g_allocations.store(0, std::memory_order_relaxed);
      player.process(input.data(), player_output.data(), options.max_block);
      assert(g_allocations.load(std::memory_order_relaxed) == 0);
      for (std::size_t pos = options.max_block; pos < input.size(); pos += options.max_block)
        player.process(input.data() + pos, player_output.data() + pos,
                       std::min<std::size_t>(options.max_block, input.size() - pos));
      assert(player.underflow_frames() == 0);
      assert(player.late_underflow_frames() == 0);

      // A raw Core instance selected with the same size is the numerical
      // reference.  The Player's zero-frame activation must leave this path
      // identical to Core's first live process while keeping activation off RT.
      auto raw_dsp = nam::get_dsp(model);
      auto* raw_slimmable = dynamic_cast<nam::SlimmableModel*>(raw_dsp.get());
      assert(raw_slimmable != nullptr);
      raw_slimmable->SetSlimmableSize(size);
      raw_dsp->ResetAndPrewarm(sample_rate, options.max_block);
      std::vector<float> raw_output(input.size(), 0.0f);
      for (std::size_t pos = 0; pos < input.size(); pos += options.max_block) {
        const std::size_t n = std::min<std::size_t>(options.max_block, input.size() - pos);
        NAM_SAMPLE* in_channels[1] = {const_cast<NAM_SAMPLE*>(input.data() + pos)};
        NAM_SAMPLE* out_channels[1] = {raw_output.data() + pos};
        raw_dsp->process(in_channels, out_channels, static_cast<int>(n));
      }
      for (std::size_t i = 0; i < input.size(); ++i)
        assert(std::abs(player_output[i] - raw_output[i]) < 1.0e-5f);
    }
  }
}

}  // namespace

int main() {
  test_equal_rate_irregular_and_in_place();
  test_rate_conversion_has_no_block_drift();
  test_rate_conversion_is_chunking_invariant();
  test_single_frame_calls_and_no_allocations();
  test_impulse_latency_and_conversion_sweep();
  test_rate_and_mono_validation();
  test_nested_stereo_container_rejected();
  test_slimmable_size_is_applied_during_prepare();
  test_full_size_default();
  test_container_threshold_is_exclusive();
  test_player_container_warms_only_final_selection();
  test_callback_exception_preserves_output();
  test_real_upstream_slimmable_model_when_available();
  std::cout << "NAM player adapter: PASS\n";
}
