// Player's zero-input skip must be invisible: output bit-identical to running the
// model on every block, across silence gaps (including -0.0) and varied blocks.
// Engine and model both at 48 kHz so Player feeds the model the caller's blocks;
// then a 44.1 kHz engine, where the model sees the resampler's output instead.
#include <cassert>
#include <cmath>
#include <cstring>
#include <iostream>
#include <memory>
#include <random>
#include <string>
#include <vector>

#include "../nam_player.h"
#include "NAM/activations.h"
#include "NAM/get_dsp.h"
#include "NAM/slimmable.h"
#include "nam_test_models.h"

namespace {

using nlohmann::json;

std::vector<float> signal_with_gaps() {
  std::vector<float> x(4 * 48000, 0.0f);
  std::mt19937 rng(3);
  std::uniform_real_distribution<float> noise(-0.05f, 0.05f);
  for (size_t i = 0; i < x.size(); i++) {
    const bool silent = (i >= 48000 && i < 2 * 48000 + 24000) || (i >= 3 * 48000 && i < 3 * 48000 + 9000);
    x[i] = silent ? ((i % 5 == 0) ? -0.0f : 0.0f) : 0.3f * std::sin(0.02f * static_cast<float>(i)) + noise(rng);
  }
  return x;
}

// Reference: the same preparation Player::prepare performs, then every block.
template <size_t N>
std::vector<float> reference(const json& model, double size, const std::vector<float>& input, const int (&blocks)[N]) {
  nam::DspLoadOptions options;
  options.prewarm = false;
  auto dsp = nam::get_dsp(model, options);
  dsp->SetPrewarmOnReset(false);
  if (auto* slim = dynamic_cast<nam::SlimmableModel*>(dsp.get())) slim->SetSlimmableSize(size);
  dsp->ResetAndPrewarm(48000.0, 256);
  float zero_in = 0.0f, zero_out = 0.0f;
  float* zi[1] = {&zero_in};
  float* zo[1] = {&zero_out};
  dsp->process(zi, zo, 0);
  return nam_test::process_blocks(*dsp, input, blocks);
}

// Forwards to a model but reports no settle length, so Player never skips it.
class NoSkip : public nam::DSP {
 public:
  explicit NoSkip(std::unique_ptr<nam::DSP> inner)
      : DSP(1, 1, inner->GetExpectedSampleRate()), inner_(std::move(inner)) {}
  void process(NAM_SAMPLE** input, NAM_SAMPLE** output, const int num_frames) override {
    inner_->process(input, output, num_frames);
  }
  void Reset(const double sample_rate, const int max_buffer_size) override {
    inner_->SetPrewarmOnReset(GetPrewarmOnReset());
    inner_->Reset(sample_rate, max_buffer_size);
  }
  void prewarm() override { inner_->prewarm(); }

 private:
  std::unique_ptr<nam::DSP> inner_;
};

// Player at a 44.1 kHz engine rate with a 48 kHz model: the skip, now behind the
// input and output resamplers, must still leave the output bit-identical.
void check_resampled(const std::string& label, const json& model) {
  static const int kBlocks[] = {32, 1, 7, 64, 35, 256, 13, 32};
  const auto input = signal_with_gaps();
  nam::DspLoadOptions options;
  options.prewarm = false;
  tmp_nam::Options popt;
  popt.engine_rate = 44100;
  popt.max_block = 256;
  tmp_nam::Player skipping(nam::get_dsp(model, options), popt);
  tmp_nam::Player full(std::make_unique<NoSkip>(nam::get_dsp(model, options)), popt);
  std::vector<float> got(input.size()), want(input.size());
  for (size_t pos = 0, b = 0; pos < input.size(); b++) {
    const size_t n = std::min<size_t>(static_cast<size_t>(kBlocks[b % std::size(kBlocks)]), input.size() - pos);
    skipping.process(input.data() + pos, got.data() + pos, n);
    full.process(input.data() + pos, want.data() + pos, n);
    pos += n;
  }
  assert(std::memcmp(got.data(), want.data(), got.size() * sizeof(float)) == 0);
  std::cout << label << " at 44.1 kHz: bit-identical, skipped " << skipping.skipped_model_frames()
            << " model frames\n";
  assert(skipping.skipped_model_frames() > 0 && full.skipped_model_frames() == 0);
}

void check(const std::string& label, const json& model, double size, bool expect_skip) {
  static const int kBlocks[] = {32, 1, 7, 64, 35, 256, 13, 32};
  const auto input = signal_with_gaps();
  nam::DspLoadOptions options;
  options.prewarm = false;
  tmp_nam::Options popt;
  popt.engine_rate = 48000;
  popt.max_block = 256;
  popt.size = size;
  tmp_nam::Player player(nam::get_dsp(model, options), popt);
  std::vector<float> out(input.size());
  size_t pos = 0, b = 0;
  while (pos < input.size()) {
    const size_t n = std::min<size_t>(static_cast<size_t>(kBlocks[b++ % std::size(kBlocks)]), input.size() - pos);
    player.process(input.data() + pos, out.data() + pos, n);
    pos += n;
  }
  const auto want = reference(model, size, input, kBlocks);
  assert(std::memcmp(out.data(), want.data(), out.size() * sizeof(float)) == 0);
  std::cout << label << ": bit-identical, skipped " << player.skipped_model_frames() << " model frames\n";
  assert(expect_skip ? player.skipped_model_frames() > 0 : player.skipped_model_frames() == 0);
}

}  // namespace

int main() {
  nam::activations::Activation::enable_fast_tanh();
  const auto a2 = nam_test::a2_container(21);
  const auto a1 = nam_test::a1_model(8, 4, 23);
#if defined(NAM_ENABLE_A2_FAST)
  check("A2 size 0 (a2_fast<3>)", a2, 0.0, true);
  check("A2 size 1 (a2_fast<8>)", a2, 1.0, true);
  check_resampled("A2 a2_fast<3>", a2["config"]["submodels"][0]["model"]);
  check_resampled("A2 a2_fast<8>", a2["config"]["submodels"][1]["model"]);
#endif
#if defined(NAM_ENABLE_A1_FAST) && defined(__aarch64__)
  check("A1 8/4 (a1_fast<8,4>)", a1, 1.0, true);
  check_resampled("A1 a1_fast<8,4>", a1);
#else
  check("A1 8/4 (generic)", a1, 1.0, false);
#endif
  // The generic WaveNet never skips (its kernels round differently per block size).
  auto generic = a1;
  generic["config"]["layers"][0]["bottleneck"] = 8;  // off the A1 fast-path shape
  check("generic WaveNet", generic, 1.0, false);
  std::cout << "NAM zero-input skip: PASS\n";
  return 0;
}
