// nam_parity_test — drives nam::DSP via libnam_core against a WAV input and
// emits a JSON report (host-side inspection, processing and benchmarking).
//
// Sub-commands:
//   inspect <model>
//   process --model <m> --input <wav> --output <wav> --sample-rate 48000
//           [--block-size 128 | --block-sequence 1,31,32,33] [--in-place 1]
//   bench   --model <m> --block-size 256 --sample-rate 48000 --iterations 1000

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <string>
#include <time.h>
#include <utility>
#include <vector>

// glibc may expose these legacy device-number macros through the C++ headers.
#ifdef major
#undef major
#endif
#ifdef minor
#undef minor
#endif

#include "NAM/dsp.h"
#include "NAM/activations.h"
#include "NAM/get_dsp.h"
#include "../nam_model_config.h"
#include <sys/utsname.h>

namespace fs = std::filesystem;

#pragma pack(push, 1)
struct WavHeader {
  char riff[4];
  uint32_t file_size;
  char wave[4];
  char fmt[4];
  uint32_t fmt_size;
  uint16_t format;
  uint16_t channels;
  uint32_t sample_rate;
  uint32_t byte_rate;
  uint16_t block_align;
  uint16_t bits_per_sample;
  char data[4];
  uint32_t data_size;
};
#pragma pack(pop)

static std::vector<float> load_wav_mono(const std::string& path, uint32_t& sample_rate_out) {
  std::ifstream fh(path, std::ios::binary);
  if (!fh) throw std::runtime_error("can't open " + path);
  WavHeader hdr{};
  fh.read(reinterpret_cast<char*>(&hdr), sizeof(hdr));
  if (std::memcmp(hdr.riff, "RIFF", 4) || std::memcmp(hdr.wave, "WAVE", 4))
    throw std::runtime_error("not a WAV: " + path);
  sample_rate_out = hdr.sample_rate;

  // This probe accepts canonical WAVs; reject extra chunks instead of treating
  // their bytes as samples.
  if (!fh || std::memcmp(hdr.fmt, "fmt ", 4) || hdr.fmt_size != 16 ||
      std::memcmp(hdr.data, "data", 4) || !hdr.channels || hdr.channels > 2 ||
      !hdr.sample_rate || !((hdr.format == 1 && (hdr.bits_per_sample == 16 || hdr.bits_per_sample == 32)) ||
                           (hdr.format == 3 && hdr.bits_per_sample == 32)) ||
      hdr.block_align != hdr.channels * (hdr.bits_per_sample / 8) ||
      hdr.data_size % hdr.block_align || hdr.data_size > 512 * 1024 * 1024)
    throw std::runtime_error("unsupported or malformed canonical WAV: " + path);
  std::vector<char> raw(hdr.data_size);
  fh.read(raw.data(), hdr.data_size);
  if (!fh) throw std::runtime_error("truncated WAV: " + path);
  const size_t bps = hdr.bits_per_sample / 8;
  const size_t n_frames = hdr.data_size / (bps * hdr.channels);
  std::vector<float> out(n_frames);

  for (size_t i = 0; i < n_frames; ++i) {
    float acc = 0.0f;
    for (uint16_t c = 0; c < hdr.channels; ++c) {
      const char* base = raw.data() + (i * hdr.channels + c) * bps;
      float s = 0.0f;
      if (hdr.format == 1 && bps == 2) {
        int16_t v;
        std::memcpy(&v, base, 2);
        s = v / 32768.0f;
      } else if (hdr.format == 3 && bps == 4) {
        std::memcpy(&s, base, 4);
      } else if (hdr.format == 1 && bps == 4) {
        int32_t v;
        std::memcpy(&v, base, 4);
        s = v / 2147483648.0f;
      } else {
        throw std::runtime_error("unsupported wav format");
      }
      acc += s;
    }
    out[i] = acc / hdr.channels;
  }
  return out;
}

static void save_wav_mono_f32(const std::string& path, const std::vector<float>& samples,
                              uint32_t sample_rate) {
  std::ofstream fh(path, std::ios::binary);
  if (!fh) throw std::runtime_error("can't write " + path);
  WavHeader hdr{};
  std::memcpy(hdr.riff, "RIFF", 4);
  std::memcpy(hdr.wave, "WAVE", 4);
  std::memcpy(hdr.fmt, "fmt ", 4);
  std::memcpy(hdr.data, "data", 4);
  hdr.fmt_size = 16;
  hdr.format = 3;  // float
  hdr.channels = 1;
  hdr.sample_rate = sample_rate;
  hdr.bits_per_sample = 32;
  hdr.block_align = 4;
  hdr.byte_rate = sample_rate * 4;
  hdr.data_size = samples.size() * 4;
  hdr.file_size = 36 + hdr.data_size;
  fh.write(reinterpret_cast<const char*>(&hdr), sizeof(hdr));
  fh.write(reinterpret_cast<const char*>(samples.data()), hdr.data_size);
  if (!fh) throw std::runtime_error("failed writing WAV: " + path);
}

static std::string get_arg(int argc, char** argv, const std::string& key,
                           const std::string& def = "") {
  for (int i = 1; i < argc - 1; ++i)
    if (key == argv[i]) return argv[i + 1];
  return def;
}

static std::vector<size_t> parse_block_sequence(const std::string& value, size_t fallback) {
  if (value.empty()) return {fallback};
  std::vector<size_t> result;
  size_t begin = 0;
  while (begin < value.size()) {
    const size_t comma = value.find(',', begin);
    const std::string token = value.substr(begin, comma == std::string::npos ? comma : comma - begin);
    if (token.empty()) throw std::runtime_error("empty block-sequence entry");
    const auto block = static_cast<size_t>(std::stoul(token));
    if (block == 0 || block > 8192) throw std::runtime_error("block sequence values must be 1..8192");
    result.push_back(block);
    if (comma == std::string::npos) break;
    begin = comma + 1;
  }
  return result;
}

static tmp_nam::ModelFile read_configured_model(int argc, char** argv, const std::string& path,
                                                int rate, int block) {
  tmp_nam::Options options;
  options.engine_rate = rate;
  options.max_block = block;
  options.resampler_quality = std::stoi(get_arg(argc, argv, "--resampler-quality", "5"));
  auto file = tmp_nam::read_model(path, options, get_arg(argc, argv, "--config", ""));
  const auto size = get_arg(argc, argv, "--size");
  if (!size.empty()) file.options.size = std::stod(size);
  const auto override_rate = get_arg(argc, argv, "--model-rate");
  if (!override_rate.empty()) file.options.model_rate_override = std::stoi(override_rate);
  return file;
}

static nlohmann::json info(const tmp_nam::ModelFile& file, const tmp_nam::Player& player,
                           const std::string& path) {
  struct utsname host{};
  uname(&host);
  return {{"model", path}, {"sha256", file.hash}, {"architecture", file.data["architecture"]},
          {"sample_rate", file.options.engine_rate}, {"model_sample_rate", player.model_rate()},
          {"size", file.options.size}, {"resampler_quality", file.options.resampler_quality},
          {"prepared_max_block", file.options.max_block},
          {"conversion_latency_frames", player.latency_frames()},
          {"implementation", player.implementation()},
          {"host_machine", host.machine}, {"host_system", host.sysname}};
}
static void emit(const nlohmann::json& data) {
  std::puts(data.dump().c_str());
  std::fflush(stdout);
}

static double thread_cpu_microseconds() {
  struct timespec value {};
  if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &value) != 0)
    throw std::runtime_error("clock_gettime(CLOCK_THREAD_CPUTIME_ID) failed");
  return static_cast<double>(value.tv_sec) * 1.0e6 + static_cast<double>(value.tv_nsec) / 1.0e3;
}

#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
static void add_dense8x8_diagnostics(nlohmann::json& data) {
  const auto stats = nam::detail::GetDense8x8Diagnostics();
  data["dense8x8"] = {{"conv1x1_calls", stats.conv1x1_calls},
                      {"conv1x1_frames", stats.conv1x1_frames},
                      {"conv1d_calls", stats.conv1d_calls},
                      {"conv1d_frames", stats.conv1d_frames},
                      {"conv1d_tap_products", stats.conv1d_tap_products}};
}
#endif

static int cmd_inspect(int argc, char** argv) {
  const std::string path = argv[2];
  auto start = std::chrono::steady_clock::now();
  auto file = read_configured_model(argc, argv, path, 44100, 256);
  auto player = tmp_nam::make_player(file);
  auto data = info(file, *player, path);
  data["load_time_seconds"] = std::chrono::duration<double>(std::chrono::steady_clock::now()-start).count();
  data["has_loudness"] = file.data.contains("metadata") && file.data["metadata"].is_object() &&
      file.data["metadata"].contains("loudness") && !file.data["metadata"]["loudness"].is_null();
  // Container thresholds and per-layer allowed_channels remain in their
  // upstream representation; the inspector does not invent new size semantics.
  data["model_config"] = file.data["config"];
  if (file.data["architecture"] == "SlimmableContainer") {
    auto thresholds = nlohmann::json::array();
    for (const auto& sub : file.data["config"]["submodels"]) thresholds.push_back(sub.at("max_value"));
    data["container_size_thresholds"] = thresholds;
    data.erase("model_config");  // nested submodels contain large weight arrays
  }
  emit(data);
  return 0;
}

static int cmd_process(int argc, char** argv) {
  const auto model_path = get_arg(argc, argv, "--model");
  const auto input_path = get_arg(argc, argv, "--input");
  const auto output_path = get_arg(argc, argv, "--output");
  const int sr = std::stoi(get_arg(argc, argv, "--sample-rate", "44100"));
  const int block = std::stoi(get_arg(argc, argv, "--block-size", "32"));
  if (block <= 0 || block > 8192) throw std::runtime_error("block size must be 1..8192");
  const auto blocks = parse_block_sequence(get_arg(argc, argv, "--block-sequence"),
                                           static_cast<size_t>(block));
  const bool in_place = get_arg(argc, argv, "--in-place", "0") == "1";
  const auto t0 = std::chrono::steady_clock::now();
  auto file = read_configured_model(argc, argv, model_path, sr, 256);
  auto player = tmp_nam::make_player(file);
  const auto t1 = std::chrono::steady_clock::now();
  uint32_t in_sr = 0;
  auto input = load_wav_mono(input_path, in_sr);
  if (in_sr != static_cast<uint32_t>(sr)) throw std::runtime_error("input WAV rate differs from --sample-rate");
  std::vector<float> output(input.size());
  const auto t2 = std::chrono::steady_clock::now();
#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
  nam::detail::ResetDense8x8Diagnostics();
#endif
  player->process(input.data(), output.data(), 0);
  size_t block_index = 0;
  for (size_t pos = 0; pos < input.size();) {
    const size_t frames = std::min(blocks[block_index++ % blocks.size()], input.size() - pos);
    if (in_place)
      player->process(input.data() + pos, input.data() + pos, frames);
    else
      player->process(input.data() + pos, output.data() + pos, frames);
    pos += frames;
    if (block_index % blocks.size() == 0)
      player->process(input.data(), output.data(), 0);
  }
  if (in_place) output = std::move(input);
  const auto t3 = std::chrono::steady_clock::now();
  save_wav_mono_f32(output_path, output, sr);
  auto data = info(file, *player, model_path);
  data["block_size"] = block;
  data["block_sequence"] = blocks;
  data["in_place"] = in_place;
  data["processed_frames"] = output.size();
  data["skipped_model_frames"] = player->skipped_model_frames();
  data["load_time_seconds"] = std::chrono::duration<double>(t1-t0).count();
  data["inference_time_seconds"] = std::chrono::duration<double>(t3-t2).count();
#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
  add_dense8x8_diagnostics(data);
#endif
  emit(data);
  return 0;
}

static int cmd_bench(int argc, char** argv) {
  const auto path = get_arg(argc, argv, "--model");
  const int sr = std::stoi(get_arg(argc, argv, "--sample-rate", "44100"));
  const int block = std::stoi(get_arg(argc, argv, "--block-size", "32"));
  const int iterations = std::stoi(get_arg(argc, argv, "--iterations", "10000"));
  if (block <= 0 || block > 8192 || iterations <= 0 || iterations > 10000000)
    throw std::runtime_error("invalid block size or iterations");
  auto file = read_configured_model(argc, argv, path, sr, 256);
  auto player = tmp_nam::make_player(file);
  std::vector<float> input(block), output(block);
  std::vector<double> times;
  std::vector<double> thread_cpu_times;
  times.reserve(iterations);
  thread_cpu_times.reserve(iterations);
#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
  nam::detail::ResetDense8x8Diagnostics();
#endif
  const double budget = block * 1e6 / sr;
  uint64_t misses = 0;
  for (int iteration = -100; iteration < iterations; ++iteration) {
    for (int i = 0; i < block; ++i)
      input[i] = static_cast<float>(0.3 * std::sin(2 * 3.141592653589793 * 220 *
                        (double(iteration + 100) * block + i) / sr));
    auto start = std::chrono::steady_clock::now();
    const double thread_cpu_start = thread_cpu_microseconds();
    player->process(input.data(), output.data(), block);
    const double thread_cpu_us = thread_cpu_microseconds() - thread_cpu_start;
    const double us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now()-start).count();
    if (iteration < 0) continue;
    times.push_back(us);
    thread_cpu_times.push_back(thread_cpu_us);
    if (us > budget) ++misses;
    for (float sample : output) if (!std::isfinite(sample)) throw std::runtime_error("non-finite NAM output");
    if ((iteration + 1) % 10000 == 0) {
      const nlohmann::json progress = {{"event", "progress"}, {"model", path},
                                      {"iterations", iteration+1}, {"deadline_misses", misses}};
      std::fprintf(stderr, "%s\n", progress.dump().c_str());
      std::fflush(stderr);
    }
  }
  std::sort(times.begin(), times.end());
  std::sort(thread_cpu_times.begin(), thread_cpu_times.end());
  auto quantile = [](const std::vector<double>& values, double q) {
    return values[static_cast<size_t>(std::ceil(q * values.size())) - 1];
  };
  auto data = info(file, *player, path);
  data["block_size"] = block;
  data["iterations"] = iterations;
  data["median_us"] = quantile(times, .5);
  data["p95_us"] = quantile(times, .95);
  data["p99_us"] = quantile(times, .99);
  data["p999_us"] = quantile(times, .999);
  data["max_us"] = times.back();
  data["median_thread_cpu_us"] = quantile(thread_cpu_times, .5);
  data["p95_thread_cpu_us"] = quantile(thread_cpu_times, .95);
  data["p99_thread_cpu_us"] = quantile(thread_cpu_times, .99);
  data["p999_thread_cpu_us"] = quantile(thread_cpu_times, .999);
  data["max_thread_cpu_us"] = thread_cpu_times.back();
  data["budget_us"] = budget;
  data["deadline_misses"] = misses;
  data["p999_share"] = quantile(times, .999) / budget;
  data["timing_scope"] = "standalone player including SRC; not a full preset or sustained hardware test";
#if defined(NAM_DENSE8X8_DIAGNOSTICS) && defined(__aarch64__)
  add_dense8x8_diagnostics(data);
#endif
  emit(data);
  return 0;
}

int main(int argc, char** argv) {
  if (argc < 2) {
    std::fprintf(stderr, "usage: %s {inspect|process|bench} [...]\n", argv[0]);
    return 2;
  }
  try {
    if (get_arg(argc, argv, "--fast-tanh", "0") == "1")
      nam::activations::Activation::enable_fast_tanh();
    const std::string command = argv[1];
    if (command == "inspect" && argc > 2) return cmd_inspect(argc, argv);
    if (command == "process") return cmd_process(argc, argv);
    if (command == "bench") return cmd_bench(argc, argv);
    throw std::runtime_error("expected inspect <model>, process, or bench");
  } catch (const std::exception& error) {
    std::fprintf(stderr, "error: %s\n", error.what());
    return 1;
  }
}
