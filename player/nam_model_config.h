#pragma once

#include "nam_player.h"
#include "nam_sha256.h"
#include "NAM/get_dsp.h"
#include "json.hpp"
#include <cmath>
#include <filesystem>
#include <fstream>
#include <stdexcept>

namespace tmp_nam {
// All functions here are control/worker-only. Hash and parse the SAME bytes so
// replacing a file cannot apply one model's settings to another model's weights.
inline std::string read_bounded(const std::filesystem::path& path, size_t limit) {
  std::ifstream stream(path, std::ios::binary);
  if (!stream) throw std::runtime_error("cannot read " + path.string());
  std::string result;
  char buffer[8192];
  while (stream) {
    stream.read(buffer, sizeof buffer);
    auto count = static_cast<size_t>(stream.gcount());
    if (result.size() + count > limit) throw std::runtime_error("file too large: " + path.string());
    result.append(buffer, count);
  }
  if (!stream.eof()) throw std::runtime_error("read error: " + path.string());
  return result;
}

struct ModelFile {
  nlohmann::json data;
  std::string hash;
  Options options;
};

inline ModelFile read_model(const std::filesystem::path& path, Options options = {},
                           const std::filesystem::path& config_path = "/data/nam/player.json") {
  auto bytes = read_bounded(path, 64 * 1024 * 1024);
  ModelFile file{nlohmann::json::parse(bytes), sha256(bytes), options};
  if (!file.data.is_object() || !file.data.contains("architecture") ||
      !file.data.contains("config") || !file.data.contains("weights"))
    throw std::runtime_error("not a NAM model: " + path.string());
  if (!config_path.empty() && config_path != "/data/nam/player.json" &&
      !std::filesystem::exists(config_path))
    throw std::runtime_error("NAM player config does not exist: " + config_path.string());
  if (!config_path.empty() && std::filesystem::exists(config_path)) {
    auto config = nlohmann::json::parse(read_bounded(config_path, 1024 * 1024));
    if (!config.is_object() || !config.contains("models") || !config["models"].is_object())
      throw std::runtime_error("NAM player config must contain a models object");
    const auto& models = config["models"];
    if (models.contains(file.hash)) {
      const auto& selected = models.at(file.hash);
      if (!selected.is_object()) throw std::runtime_error("NAM model settings must be an object");
      for (auto it = selected.begin(); it != selected.end(); ++it)
        if (it.key() != "size" && it.key() != "sample_rate_hz" && it.key() != "output_gain")
          throw std::runtime_error("unknown NAM model setting: " + it.key());
      if (selected.contains("size")) file.options.size = selected.at("size").get<double>();
      if (selected.contains("output_gain"))
        file.options.output_gain = selected.at("output_gain").get<double>();
      if (selected.contains("sample_rate_hz")) {
        const auto& rate = selected.at("sample_rate_hz");
        if (!rate.is_number_integer() || (rate.get<double>() != 44100 && rate.get<double>() != 48000))
          throw std::runtime_error("sample_rate_hz must be 44100 or 48000");
        file.options.model_rate_override = rate.get<int>();
      }
    }
  }
  if (!std::isfinite(file.options.size) || file.options.size < 0 || file.options.size > 1)
    throw std::runtime_error("NAM size must be between 0 and 1");
  // An override documents missing metadata; it must never retime known weights.
  if (file.options.model_rate_override && file.data.contains("sample_rate") &&
      file.data["sample_rate"].is_number() && file.data["sample_rate"].get<double>() > 0 &&
      file.data["sample_rate"].get<double>() != file.options.model_rate_override)
    throw std::runtime_error("sample_rate_hz conflicts with NAM metadata");
  return file;
}

inline void validate_container_rates(const nlohmann::json& model, double rate, unsigned depth = 0) {
  if (depth > 16) throw std::runtime_error("NAM container nesting exceeds 16");
  if (model.contains("sample_rate") && model["sample_rate"].is_number()) {
    const double declared = model["sample_rate"].get<double>();
    if (declared > 0 && rate > 0 && declared != rate)
      throw std::runtime_error("NAM container child rate conflicts with effective model rate");
  }
  if (model.value("architecture", "") == "SlimmableContainer")
    for (const auto& child : model.at("config").at("submodels"))
      validate_container_rates(child.at("model"), rate, depth + 1);
}

inline std::unique_ptr<Player> make_player(const ModelFile& file) {
  const double rate = file.options.model_rate_override ? file.options.model_rate_override :
      file.data.value("sample_rate", -1.0);
  validate_container_rates(file.data, rate);
  nam::DspLoadOptions load_options;
  load_options.prewarm = false;
  return std::make_unique<Player>(nam::get_dsp(file.data, load_options), file.options);
}
}  // namespace tmp_nam
