#include "nam_player.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>
#include <stdexcept>
#include <string>

#include <speex/speex_resampler.h>

#include "NAM/slimmable.h"

namespace tmp_nam {
namespace {

constexpr int kRate44100 = 44100;
constexpr int kRate48000 = 48000;

bool supported_rate(int rate) {
  return rate == kRate44100 || rate == kRate48000;
}

std::size_t ceil_ratio(std::size_t n, int numerator, int denominator) {
  const long double value = static_cast<long double>(n) * numerator / denominator;
  if (value > static_cast<long double>(std::numeric_limits<std::size_t>::max() - 64))
    throw std::invalid_argument("NAM player buffer size overflow");
  return static_cast<std::size_t>(std::ceil(value));
}

std::size_t checked_add(std::size_t a, std::size_t b) {
  if (b > std::numeric_limits<std::size_t>::max() - a)
    throw std::invalid_argument("NAM player buffer size overflow");
  return a + b;
}

}  // namespace

struct Player::Resampler {
  SpeexResamplerState* state = nullptr;

  Resampler(int input_rate, int output_rate, int quality) {
    int error = RESAMPLER_ERR_SUCCESS;
    state = speex_resampler_init(1, static_cast<spx_uint32_t>(input_rate),
                                 static_cast<spx_uint32_t>(output_rate), quality,
                                 &error);
    if (!state || error != RESAMPLER_ERR_SUCCESS) {
      if (state) speex_resampler_destroy(state);
      state = nullptr;
      throw std::runtime_error("failed to create Speex resampler: " +
                               std::string(speex_resampler_strerror(error)));
    }
  }

  ~Resampler() {
    if (state) speex_resampler_destroy(state);
  }

  Resampler(const Resampler&) = delete;
  Resampler& operator=(const Resampler&) = delete;
};

Player::Player(std::unique_ptr<nam::DSP> dsp)
    : Player(std::move(dsp), Options{}) {}

Player::Player(std::unique_ptr<nam::DSP> dsp, Options options)
    : dsp_(std::move(dsp)), options_(options) {
  if (!dsp_) throw std::invalid_argument("NAM player requires a DSP");
  prepare();
}

Player::~Player() = default;

void Player::prepare() {
  prepared_ = false;
  if (!supported_rate(options_.engine_rate))
    throw std::invalid_argument("NAM player supports engine rates 44100 and 48000 Hz");
  if (options_.max_block <= 0)
    throw std::invalid_argument("NAM player max_block must be positive");
  if (options_.resampler_quality != 5 && options_.resampler_quality != 6)
    throw std::invalid_argument("NAM player resampler quality must be 5 or 6");
  if (!std::isfinite(options_.size) || options_.size < 0.0 || options_.size > 1.0)
    throw std::invalid_argument("NAM player size must be between 0 and 1");
  if (!std::isfinite(options_.output_gain) || options_.output_gain < 0.0 ||
      options_.output_gain > 8.0)
    throw std::invalid_argument("NAM player output_gain must be between 0 and 8");
  if (options_.model_rate_override != 0 &&
      !supported_rate(options_.model_rate_override))
    throw std::invalid_argument("NAM player model rate override must be 44100 or 48000 Hz");

  const int channels_in = dsp_->NumInputChannels();
  const int channels_out = dsp_->NumOutputChannels();
  if (channels_in != 1 || channels_out != 1)
    throw std::invalid_argument("NAM player requires a mono-in/mono-out DSP");

  const double declared_rate = dsp_->GetExpectedSampleRate();
  if (options_.model_rate_override != 0) {
    if (std::isfinite(declared_rate) && declared_rate > 0.0 &&
        declared_rate != static_cast<double>(options_.model_rate_override))
      throw std::invalid_argument("NAM model rate override conflicts with DSP metadata");
    model_rate_ = options_.model_rate_override;
  } else {
    if (!std::isfinite(declared_rate) || declared_rate <= 0.0 ||
        std::floor(declared_rate) != declared_rate ||
        declared_rate > static_cast<double>(std::numeric_limits<int>::max())) {
      throw std::invalid_argument(
          "NAM DSP has no known sample rate; supply model_rate_override");
    }
    model_rate_ = static_cast<int>(declared_rate);
    if (!supported_rate(model_rate_))
      throw std::invalid_argument("NAM DSP sample rate must be 44100 or 48000 Hz");
  }

  // Suppress constructor/sizing resets so only the selected final model is
  // warmed by ResetAndPrewarm below.  Container and WaveNet wrappers propagate
  // this policy to their children.
  dsp_->SetPrewarmOnReset(false);

  // SetSlimmableSize is intentionally applied before ResetAndPrewarm.  A
  // container may replace its active submodel and Reset must size that model's
  // buffers.  This function is expected to run on the worker/control thread.
  if (auto* slimmable = dynamic_cast<nam::SlimmableModel*>(dsp_.get()))
    slimmable->SetSlimmableSize(options_.size);

  engine_capacity_ = static_cast<std::size_t>(options_.max_block);
  model_block_ = engine_capacity_;
  if (model_rate_ != options_.engine_rate)
    model_block_ = checked_add(
        ceil_ratio(engine_capacity_, model_rate_, options_.engine_rate), 64);
  model_capacity_ = std::max(engine_capacity_, model_block_);

  // Speex can retain a small filter tail at either side of a conversion.  The
  // extra block-sized slack keeps process_float from ever needing a retry with
  // a newly allocated buffer.  The actual delay is reported from the states.
  const std::size_t scratch_capacity = checked_add(model_capacity_, 128);
  output_capacity_ = std::max<std::size_t>(4096, checked_add(scratch_capacity, 256));

  engine_input_ = std::make_unique<float[]>(engine_capacity_);
  model_input_ = std::make_unique<float[]>(scratch_capacity);
  model_output_ = std::make_unique<float[]>(scratch_capacity);
  engine_output_ = std::make_unique<float[]>(scratch_capacity);
  output_fifo_ = std::make_unique<float[]>(output_capacity_);

  input_resampler_.reset();
  output_resampler_.reset();
  if (model_rate_ != options_.engine_rate) {
    input_resampler_ = std::make_unique<Resampler>(
        options_.engine_rate, model_rate_, options_.resampler_quality);
    output_resampler_ = std::make_unique<Resampler>(
        model_rate_, options_.engine_rate, options_.resampler_quality);
  }

  // The Core reset is the final preparation step.  Its max buffer size is in
  // model frames, so it includes the ratio conversion slack.
  if (model_capacity_ > static_cast<std::size_t>(std::numeric_limits<int>::max()))
    throw std::invalid_argument("NAM player model block is too large");
  dsp_->ResetAndPrewarm(static_cast<double>(model_rate_),
                        static_cast<int>(model_capacity_));

  // SlimmableWavenet publishes a freshly rebuilt model into a pending slot;
  // its process() takes that slot before touching audio.  Consume the pending
  // slot here, on the worker, with a zero-frame call.  This leaves the first
  // live callback free of model activation/destruction work while preserving
  // the model state (zero frames performs no DSP processing).
  NAM_SAMPLE activation_input_buffer[1] = {0};
  NAM_SAMPLE activation_output_buffer[1] = {0};
  NAM_SAMPLE* activation_input[1] = {activation_input_buffer};
  NAM_SAMPLE* activation_output[1] = {activation_output_buffer};
  dsp_->process(activation_input, activation_output, 0);

  latency_frames_ = 0;
  if (input_resampler_) {
    const auto input_latency = speex_resampler_get_input_latency(input_resampler_->state);
    const auto output_latency = speex_resampler_get_output_latency(output_resampler_->state);
    // Speex reports input latency in input-stream frames and output latency
    // in output-stream frames.  The first resampler's input is the engine
    // stream; the second resampler's output is the engine stream.  Both
    // therefore add directly in the engine timeline.
    const long double frames = static_cast<long double>(input_latency) +
                               static_cast<long double>(output_latency);
    if (!std::isfinite(static_cast<double>(frames)) || frames < 0.0L ||
        frames > static_cast<long double>(std::numeric_limits<std::size_t>::max()))
      throw std::runtime_error("invalid Speex resampler latency");
    latency_frames_ = static_cast<std::size_t>(std::ceil(frames));
  }

  // Margin over the receptive field: twice the settle length plus one full block.
  const long settle = dsp_->ZeroInputSettleSamples();
  zero_skip_after_ = settle >= 0 ? 2 * settle + static_cast<long>(model_capacity_) : -1;
  zero_run_ = 0;
  zero_skipping_ = false;
  zero_output_ = 0.0f;

  reset_fifo();
  counters_ = {};
  prepared_ = true;
}

double Player::latency_seconds() const noexcept {
  return static_cast<double>(latency_frames_) / options_.engine_rate;
}

void Player::reset_fifo() noexcept {
  fifo_read_ = 0;
  fifo_count_ = 0;
}

void Player::push_output(const float* data, std::size_t frames) {
  if (frames > output_capacity_ - fifo_count_)
    throw std::runtime_error("NAM player output FIFO overflow");
  std::size_t write = (fifo_read_ + fifo_count_) % output_capacity_;
  const std::size_t first = std::min(frames, output_capacity_ - write);
  std::copy_n(data, first, output_fifo_.get() + write);
  if (frames > first) std::copy_n(data + first, frames - first, output_fifo_.get());
  fifo_count_ += frames;
}

std::size_t Player::pop_output(float* data, std::size_t frames) noexcept {
  const std::size_t count = std::min(frames, fifo_count_);
  const std::size_t first = std::min(count, output_capacity_ - fifo_read_);
  std::copy_n(output_fifo_.get() + fifo_read_, first, data);
  if (count > first) std::copy_n(output_fifo_.get(), count - first, data + first);
  fifo_read_ = (fifo_read_ + count) % output_capacity_;
  fifo_count_ -= count;
  return count;
}

void Player::run_model(float* input, float* output, std::size_t frames) {
  bool silent = zero_skip_after_ >= 0;
  for (std::size_t i = 0; silent && i < frames; ++i) silent = input[i] == 0.0f;
  if (!silent) {
    zero_run_ = 0;
    zero_skipping_ = false;
  } else if (zero_skipping_) {
    std::fill_n(output, frames, zero_output_);
    counters_.skipped_model_frames += frames;
    return;
  }
  float* in_channels[1] = {input};
  float* out_channels[1] = {output};
  dsp_->process(in_channels, out_channels, static_cast<int>(frames));
  if (!silent) return;
  zero_run_ += static_cast<long>(frames);
  if (zero_run_ < zero_skip_after_ || frames == 0) return;
  // Engage only once a whole block has settled to one bit-identical value.
  for (std::size_t i = 1; i < frames; ++i)
    if (std::memcmp(&output[i], &output[0], sizeof(float)) != 0) return;
  zero_output_ = output[0];
  zero_skipping_ = true;
}

void Player::process_chunk(const float* input, float* output, std::size_t frames) {
  std::copy_n(input, frames, engine_input_.get());

  if (!input_resampler_) {
    run_model(engine_input_.get(), model_output_.get(), frames);
    std::copy_n(model_output_.get(), frames, output);
    counters_.model_frames += frames;
    return;
  }

  std::size_t input_offset = 0;
  while (input_offset < frames) {
    spx_uint32_t input_count = static_cast<spx_uint32_t>(frames - input_offset);
    spx_uint32_t model_count = static_cast<spx_uint32_t>(model_capacity_);
    const int err = speex_resampler_process_float(
        input_resampler_->state, 0, engine_input_.get() + input_offset, &input_count,
        model_input_.get(), &model_count);
    if (err != RESAMPLER_ERR_SUCCESS)
      throw std::runtime_error("input Speex resampler failed");
    input_offset += input_count;
    if (input_count == 0 && model_count == 0)
      throw std::runtime_error("input Speex resampler made no progress");

    std::size_t model_offset = 0;
    while (model_offset < model_count) {
      const std::size_t n = std::min(model_count - model_offset, model_capacity_);
      run_model(model_input_.get() + model_offset, model_output_.get(), n);
      counters_.model_frames += n;

      std::size_t out_offset = 0;
      while (out_offset < n) {
        spx_uint32_t in_n = static_cast<spx_uint32_t>(n - out_offset);
        spx_uint32_t out_n = static_cast<spx_uint32_t>(engine_capacity_ + 128);
        const int out_err = speex_resampler_process_float(
            output_resampler_->state, 0, model_output_.get() + out_offset, &in_n,
            engine_output_.get(), &out_n);
        if (out_err != RESAMPLER_ERR_SUCCESS)
          throw std::runtime_error("output Speex resampler failed");
        out_offset += in_n;
        if (out_n) push_output(engine_output_.get(), out_n);
        if (in_n == 0 && out_n == 0)
          throw std::runtime_error("output Speex resampler made no progress");
      }
      model_offset += n;
    }
  }

  // Do not commit a partial callback if conversion ever underflows.
  if (fifo_count_ < frames) {
    const std::size_t missing = frames - fifo_count_;
    counters_.underflow_frames += missing;
    counters_.late_underflow_frames += missing;
    throw std::runtime_error("NAM player output underflow");
  }
  pop_output(output, frames);
}

void Player::process(const float* input, float* output, std::size_t frames) {
  if (!prepared_) throw std::logic_error("NAM player is not prepared");
  if (frames == 0) return;
  if (!input || !output) throw std::invalid_argument("NAM player buffers must be non-null");

  std::size_t offset = 0;
  while (offset < frames) {
    const std::size_t n = std::min(frames - offset, engine_capacity_);
    process_chunk(input + offset, output + offset, n);
    offset += n;
  }
  // Explicit makeup gain only; the == 1.0 path is bit-identical to the model.
  // RT-safe: one multiply per sample, no branches in the loop body.
  const float gain = static_cast<float>(options_.output_gain);
  if (gain != 1.0f) {
    for (std::size_t i = 0; i < frames; ++i) output[i] *= gain;
  }
  ++counters_.process_calls;
  counters_.engine_frames += frames;
}

}  // namespace tmp_nam
