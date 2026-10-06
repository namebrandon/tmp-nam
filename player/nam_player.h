#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>

#include "NAM/dsp.h"

namespace tmp_nam {

// A prepared, single-channel NAM player.  Construction and prepare() belong
// on a control/worker thread; process() is deliberately a small, allocation
// free operation suitable for an audio callback.
class Player final {
 public:
  struct Options {
    int engine_rate = 44100;
    int max_block = 256;
    int resampler_quality = 5;
    double size = 1.0;
    int model_rate_override = 0;
    // Explicit output makeup gain (default 1.0 = bit-identical to the model).
    // NAM captures typically land ~12 dB below stock blocks; set per model
    // hash in player.json instead of baking gain into anyone's capture.
    double output_gain = 1.0;
  };

  struct Counters {
    uint64_t process_calls = 0;
    uint64_t engine_frames = 0;
    uint64_t model_frames = 0;
    uint64_t underflow_frames = 0;
    uint64_t late_underflow_frames = 0;
    // Model frames answered from the settled zero-input output instead of the network.
    uint64_t skipped_model_frames = 0;
  };

  explicit Player(std::unique_ptr<nam::DSP> dsp);
  Player(std::unique_ptr<nam::DSP> dsp, Options options);
  ~Player();

  Player(const Player&) = delete;
  Player& operator=(const Player&) = delete;
  Player(Player&&) = delete;
  Player& operator=(Player&&) = delete;

  // Re-prepares the owned model.  This is a worker-thread operation and is
  // useful when a staged model has had its slimmable size selected.
  void prepare();

  // Exactly frames are written.  input and output may point at the same
  // storage.  For a rate conversion, the resampler phases and the small
  // output FIFO persist across calls. If a long call throws, earlier completed
  // chunks remain committed; the failing chunk does not overwrite its output.
  void process(const float* input, float* output, std::size_t frames);

  int model_rate() const noexcept { return model_rate_; }
  // Which Core implementation runs the network ("generic", "a2_fast<8>", ...).
  // Control/worker thread only, like prepare().
  const char* implementation() const { return dsp_->ImplementationName(); }
  std::size_t latency_frames() const noexcept { return latency_frames_; }
  double latency_seconds() const noexcept;
  const Counters& counters() const noexcept { return counters_; }
  uint64_t process_calls() const noexcept { return counters_.process_calls; }
  uint64_t engine_frames() const noexcept { return counters_.engine_frames; }
  uint64_t model_frames() const noexcept { return counters_.model_frames; }
  uint64_t underflow_frames() const noexcept { return counters_.underflow_frames; }
  uint64_t late_underflow_frames() const noexcept {
    return counters_.late_underflow_frames;
  }
  uint64_t skipped_model_frames() const noexcept { return counters_.skipped_model_frames; }

 private:
  struct Resampler;

  std::unique_ptr<nam::DSP> dsp_;
  Options options_;
  std::unique_ptr<Resampler> input_resampler_;
  std::unique_ptr<Resampler> output_resampler_;

  int model_rate_ = 0;
  std::size_t model_block_ = 0;
  std::size_t latency_frames_ = 0;
  bool prepared_ = false;

  // These buffers are all sized by prepare().  No allocation operation is made
  // from process().  The FIFO is a ring so a long caller buffer does not make
  // the audio path depend on its total length.
  std::unique_ptr<float[]> engine_input_;
  std::unique_ptr<float[]> model_input_;
  std::unique_ptr<float[]> model_output_;
  std::unique_ptr<float[]> engine_output_;
  std::unique_ptr<float[]> output_fifo_;
  std::size_t engine_capacity_ = 0;
  std::size_t model_capacity_ = 0;
  std::size_t output_capacity_ = 0;
  std::size_t fifo_read_ = 0;
  std::size_t fifo_count_ = 0;
  Counters counters_;

  // Zero-input steady state. A feed-forward model that has seen at least
  // zero_skip_after_ consecutive zero-valued input frames outputs one exact constant
  // until its input changes, so run_model() repeats that constant instead of running
  // the network. The firmware feeds exact zeros to the inactive bank on one core.
  // -1 disables the skip (recurrent or unknown models).
  long zero_skip_after_ = -1;
  long zero_run_ = 0;
  bool zero_skipping_ = false;
  float zero_output_ = 0.0f;

  void reset_fifo() noexcept;
  void push_output(const float* data, std::size_t frames);
  std::size_t pop_output(float* data, std::size_t frames) noexcept;
  void process_chunk(const float* input, float* output, std::size_t frames);
  void run_model(float* input, float* output, std::size_t frames);
};

// Kept at namespace scope so registry/configuration code can construct an
// Options value without depending on the concrete Player spelling.
using Options = Player::Options;

}  // namespace tmp_nam
