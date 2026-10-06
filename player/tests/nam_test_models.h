// Synthetic NAM models for the native tests: trainer architectures (A1 WaveNet sizes,
// A2 3/8-channel SlimmableContainer) with seeded random weights, so no capture is tracked.
#pragma once

#include <algorithm>
#include <iterator>
#include <random>
#include <string>
#include <vector>

#include "NAM/dsp.h"
#include "json.hpp"

namespace nam_test {

using nlohmann::json;

inline std::vector<float> random_weights(size_t n, float stddev, unsigned seed) {
  std::mt19937 rng(seed);
  std::normal_distribution<float> dist(0.0f, stddev);
  std::vector<float> weights(n);
  for (auto& w : weights) w = dist(rng);
  return weights;
}

inline json a1_config(int c1, int c2, const std::vector<int>& d1, const std::vector<int>& d2, bool hb1, bool hb2) {
  auto layer = [](int input, int channels, int head, const std::vector<int>& d, bool hb) {
    return json{{"input_size", input}, {"condition_size", 1}, {"head_size", head}, {"channels", channels},
                {"kernel_size", 3}, {"dilations", d}, {"activation", "Tanh"}, {"gated", false}, {"head_bias", hb}};
  };
  return json{{"layers", {layer(1, c1, c2, d1, hb1), layer(c1, c2, 1, d2, hb2)}}, {"head", nullptr}, {"head_scale", 0.02}};
}

inline size_t a1_weight_count(const json& config) {
  size_t total = 1;  // head_scale
  for (const auto& la : config["layers"]) {
    const size_t in = la["input_size"], c = la["channels"], h = la["head_size"], n = la["dilations"].size();
    total += in * c + n * (3 * c * c + c + c + c * c + c) + c * h + (la["head_bias"].get<bool>() ? h : 0);
  }
  return total;
}

// Plain A1 WaveNet (standard 16/8, lite 12/6, feather 8/4, nano 4/2).
inline json a1_model(int c1, int c2, unsigned seed) {
  const std::vector<int> d1 = {1, 2, 4, 8, 16, 32, 64, 128, 256, 512};
  const std::vector<int> d2 = {1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 3};
  auto config = a1_config(c1, c2, d1, d2, false, true);
  return json{{"version", "0.5.4"}, {"architecture", "WaveNet"}, {"config", config},
              {"weights", random_weights(a1_weight_count(config), 0.3f, seed)}, {"sample_rate", 48000}};
}

// One A2 child: a single 23-layer array, LeakyReLU(0.01), 1x1 layers, 16-tap head.
inline json a2_model(int channels, unsigned seed) {
  const std::vector<int> dilations = {1, 3, 7, 17, 41, 101, 239, 1, 3, 7, 17, 41, 101, 239, 1, 13, 1, 3, 7, 17, 41, 101, 239};
  const std::vector<int> kernels = {6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 15, 15, 6, 6, 6, 6, 6, 6, 6};
  const size_t n = dilations.size(), c = static_cast<size_t>(channels);
  json film = {{"active", false}, {"shift", true}, {"groups", 1}};
  json layer = {{"input_size", 1}, {"condition_size", 1}, {"channels", channels}, {"bottleneck", channels},
                {"kernel_sizes", kernels}, {"dilations", dilations},
                {"activation", std::vector<json>(n, {{"type", "LeakyReLU"}, {"negative_slope", 0.01}})},
                {"gating_mode", std::vector<std::string>(n, "none")},
                {"secondary_activation", std::vector<json>(n, nullptr)},
                {"groups_input", 1}, {"groups_input_mixin", 1}, {"slimmable", nullptr},
                {"layer1x1", {{"active", true}, {"groups", 1}}},
                {"head1x1", {{"active", false}, {"out_channels", 1}, {"groups", 1}}},
                {"head", {{"kernel_size", 16}, {"out_channels", 1}, {"bias", true}}}};
  for (const char* key : {"conv_pre_film", "conv_post_film", "input_mixin_pre_film", "input_mixin_post_film",
                          "activation_pre_film", "activation_post_film", "layer1x1_post_film", "head1x1_post_film"})
    layer[key] = film;
  size_t taps = 0;
  for (int k : kernels) taps += static_cast<size_t>(k);
  // Rechannel, dilated convs + biases, input mixins, 1x1 layers + biases, head + bias, head_scale.
  const size_t count = c + c * c * taps + c * n + c * n + (c * c + c) * n + 16 * c + 1 + 1;
  json config = {{"layers", {layer}}, {"head", nullptr}, {"head_scale", 0.005}};
  return json{{"version", "0.7.0"}, {"architecture", "WaveNet"}, {"config", config},
              {"weights", random_weights(count, 0.1f, seed)}, {"sample_rate", 48000}};
}

// Runs an already prepared DSP over `in`, cycling through `blocks` for the block sizes.
template <size_t N>
inline std::vector<float> process_blocks(nam::DSP& dsp, std::vector<float> in, const int (&blocks)[N]) {
  std::vector<float> out(in.size());
  for (size_t pos = 0, b = 0; pos < in.size(); b++) {
    const int n = static_cast<int>(std::min<size_t>(static_cast<size_t>(blocks[b % N]), in.size() - pos));
    float* ip[1] = {in.data() + pos};
    float* op[1] = {out.data() + pos};
    dsp.process(ip, op, n);
    pos += static_cast<size_t>(n);
  }
  return out;
}

// A2 SlimmableContainer: 3 channels up to size 0.5, 8 channels up to 1.
inline json a2_container(unsigned seed) {
  json submodels = {{{"max_value", 0.5}, {"model", a2_model(3, seed)}}, {{"max_value", 1}, {"model", a2_model(8, seed + 1)}}};
  return json{{"version", "0.7.0"}, {"architecture", "SlimmableContainer"}, {"config", {{"submodels", submodels}}}, {"weights", json::array()},
              {"sample_rate", 48000}};
}

}  // namespace nam_test
