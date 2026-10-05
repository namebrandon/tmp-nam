#include "../nam_model_config.h"
#include <cassert>
#include <chrono>
#include <iostream>

int main() {
  using namespace tmp_nam;
  assert(sha256("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  assert(sha256("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  const auto dir = std::filesystem::temp_directory_path() /
      ("tmp-nam-config-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
  std::filesystem::create_directory(dir);
  struct Cleanup { std::filesystem::path path; ~Cleanup() { std::filesystem::remove_all(path); } } cleanup{dir};
  const auto model = dir / "model.nam", config = dir / "player.json";
  auto write = [](const auto& path, const auto& value) { std::ofstream(path) << value; };
  const std::string bytes = R"({"architecture":"Linear","config":{"receptive_field":1,"bias":false},"weights":[1.0],"sample_rate":48000,"version":"0.5.4"})";
  write(model, bytes);
  auto file = read_model(model, {}, "");
  assert(file.hash == sha256(bytes) && file.options.size == 1);
  write(config, R"({"models":{}})");
  auto defaults = read_model(model, {}, config);
  assert(defaults.options.size == 1 && defaults.options.resampler_quality == 5);
  nlohmann::json settings = {{"models", {{file.hash, {{"size", .5}, {"sample_rate_hz", 48000}}}}}};
  write(config, settings.dump());
  assert(read_model(model, {}, config).options.size == .5);
  auto rejects = [&](const nlohmann::json& value) {
    write(config, value.dump());
    bool failed = false;
    try { (void)read_model(model, {}, config); } catch (const std::exception&) { failed = true; }
    assert(failed);
  };
  auto& selected = settings["models"][file.hash];
  selected["sample_rate_hz"] = 44100;
  rejects(settings);
  selected["sample_rate_hz"] = 48000.5;
  rejects(settings);
  selected["sample_rate_hz"] = 0;
  rejects(settings);
  selected.erase("sample_rate_hz");
  selected["size"] = 1.01;
  rejects(settings);
  selected["size"] = .5;
  selected["typo"] = true;
  rejects(settings);
  rejects(nlohmann::json::array());
  selected.erase("typo");
  write(config, settings.dump());
  // Exact bytes own their settings: even an equivalent JSON reserialization
  // gets the full-size default until its new hash is explicitly configured.
  write(model, file.data.dump(2));
  assert(read_model(model, {}, config).options.size == 1);
  nlohmann::json child = file.data;
  nlohmann::json container = {{"architecture", "SlimmableContainer"}, {"sample_rate", -1},
      {"config", {{"submodels", {{{"max_value", 1.0}, {"model", child}}}}}}};
  validate_container_rates(container, 48000);
  bool rate_failed = false;
  try { validate_container_rates(container, 44100); }
  catch (const std::exception&) { rate_failed = true; }
  assert(rate_failed);
  write(model, "{}");
  bool failed = false;
  try { (void)read_model(model, {}, ""); } catch (const std::exception&) { failed = true; }
  assert(failed);
  std::cout << "model config tests passed\n";
}
