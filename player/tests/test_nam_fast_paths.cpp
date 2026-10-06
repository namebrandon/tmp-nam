// Fast-path coverage: the fused A1 WaveNet (all four trainer sizes) and the A2
// fast path against the generic WaveNet, plus the A1 shape detector.
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstring>
#include <iostream>
#include <random>
#include <string>
#include <vector>

#include "NAM/dsp.h"
#include "NAM/get_dsp.h"
#include "NAM/slimmable.h"
#include "NAM/wavenet/a2_fast.h"
#include "NAM/wavenet/model.h"
#include "nam_test_models.h"

namespace {

using nlohmann::json;
using nam_test::a1_model;

std::vector<float> test_signal(size_t n) {
  std::vector<float> x(n);
  std::mt19937 rng(7);
  std::uniform_real_distribution<float> noise(-0.05f, 0.05f);
  for (size_t i = 0; i < n; i++)
    x[i] = 0.3f * std::sin(0.031f * static_cast<float>(i)) + 0.2f * std::sin(0.0047f * static_cast<float>(i)) + noise(rng);
  return x;
}

std::vector<float> run(nam::DSP& dsp, const std::vector<float>& input) {
  static const int kBlocks[] = {1, 7, 32, 35, 64, 13, 34, 256, 3, 32};
  dsp.ResetAndPrewarm(48000.0, 256);
  return nam_test::process_blocks(dsp, input, kBlocks);
}

void expect_close(const std::vector<float>& a, const std::vector<float>& b, const std::string& label) {
  float peak = 0.0f, diff = 0.0f;
  for (size_t i = 0; i < a.size(); i++) {
    assert(std::isfinite(a[i]) && std::isfinite(b[i]));
    peak = std::max(peak, std::fabs(b[i]));
    diff = std::max(diff, std::fabs(a[i] - b[i]));
  }
  std::cout << label << " max diff " << diff << " (peak " << peak << ")\n";
  // Same math, different float order: rounding-level only.
  assert(peak > 1e-4f && diff <= 1e-3f * peak + 1e-6f);
}

std::unique_ptr<nam::DSP> generic(const json& model) {
  auto config = nam::wavenet::parse_config_json(model["config"], model.value("sample_rate", 48000.0));
  return config.create(model["weights"].get<std::vector<float>>(), 48000.0);
}

}  // namespace

int main() {
  nam::activations::Activation::enable_fast_tanh();
  const auto input = test_signal(48000);

#if defined(__aarch64__) && defined(NAM_ENABLE_A1_FAST)
  const int sizes[4][2] = {{16, 8}, {12, 6}, {8, 4}, {4, 2}};
  for (const auto& s : sizes) {
    const auto model = a1_model(s[0], s[1], 100u + static_cast<unsigned>(s[0]));
    assert(nam::wavenet::a1_fast::is_a1_shape(model["config"]));
    auto fast = nam::get_dsp(model);
    const std::string expected = "a1_fast<" + std::to_string(s[0]) + "," + std::to_string(s[1]) + ">";
    assert(fast->ImplementationName() == expected);
    assert(fast->ZeroInputSettleSamples() > 0);
    auto slow = generic(model);
    assert(std::strcmp(slow->ImplementationName(), "generic") == 0);
    assert(slow->ZeroInputSettleSamples() == -1);
    expect_close(run(*fast, input), run(*slow, input), "A1 " + expected);
  }
  // The detector takes only the plain A1 layout; anything else stays generic.
  auto rejects = [](json config) { assert(!nam::wavenet::a1_fast::is_a1_shape(config)); };
  const auto base = a1_model(16, 8, 1)["config"];
  { auto c = base; c["layers"][0]["bottleneck"] = 16; rejects(c); }
  { auto c = base; c["layers"][0]["gated"] = true; rejects(c); }
  { auto c = base; c["layers"][1]["activation"] = "ReLU"; rejects(c); }
  { auto c = base; c["layers"][0]["kernel_size"] = 2; rejects(c); }
  { auto c = base; c["layers"][0]["channels"] = 10; c["layers"][0]["head_size"] = 8; c["layers"][1]["input_size"] = 10; rejects(c); }
  { auto c = base; c["condition_dsp"] = nullptr; rejects(c); }
  { auto c = base; c["layers"][0]["head_size"] = 4; rejects(c); }
  { auto c = base; c["layers"].push_back(c["layers"][1]); rejects(c); }
  std::cout << "A1 detector: PASS\n";
#else
  std::cout << "A1 fast path not built on this target: skipped\n";
#endif

#if defined(NAM_ENABLE_A2_FAST)
  // A2 SlimmableContainer: each child against the generic WaveNet.
  const auto container = nam_test::a2_container(11);
  const char* names[2] = {"a2_fast<3>", "a2_fast<8>"};
  for (int child = 0; child < 2; child++) {
    const auto& sub = container["config"]["submodels"][child]["model"];
    auto fast = nam::get_dsp(sub);
    assert(std::strcmp(fast->ImplementationName(), names[child]) == 0);
    auto slow = generic(sub);
    expect_close(run(*fast, input), run(*slow, input), std::string("A2 ") + names[child]);
  }
  auto whole = nam::get_dsp(container);
  dynamic_cast<nam::SlimmableModel&>(*whole).SetSlimmableSize(1.0);
  whole->ResetAndPrewarm(48000.0, 256);
  assert(std::strcmp(whole->ImplementationName(), "a2_fast<8>") == 0);
#endif
  std::cout << "NAM fast paths: PASS\n";
  return 0;
}
