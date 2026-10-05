#pragma once

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>

namespace tmp_nam {

// Callback utilization in 0.1% bins. observe() performs one relaxed atomic
// increment and is called only when TMP_NAM_PROFILE=1. Snapshotting belongs to
// the telemetry thread.
class UtilizationHistogram final {
 public:
  static constexpr size_t kMaxPermille = 2000;

  UtilizationHistogram() noexcept {
    for (auto& bin : bins_) bin.store(0, std::memory_order_relaxed);
  }

  struct Snapshot {
    uint64_t samples = 0;
    uint64_t overflow = 0;
    size_t p999_permille_upper = 0;
    bool p999_overflow = false;
  };

  void observe(uint64_t elapsed_ns, size_t frames, uint64_t sample_rate) noexcept {
    if (!frames || !sample_rate) return;
    const uint64_t denominator = static_cast<uint64_t>(frames) * 1000000ULL;
    const uint64_t numerator = elapsed_ns * sample_rate;
    const uint64_t permille = (numerator + denominator - 1) / denominator;
    if (permille > kMaxPermille) {
      overflow_.fetch_add(1, std::memory_order_relaxed);
    } else {
      bins_[static_cast<size_t>(permille)].fetch_add(1, std::memory_order_relaxed);
    }
  }

  Snapshot snapshot() const noexcept {
    Snapshot result;
    std::array<uint64_t, kMaxPermille + 1> counts{};
    for (size_t i = 0; i < bins_.size(); ++i) {
      counts[i] = bins_[i].load(std::memory_order_relaxed);
      result.samples += counts[i];
    }
    result.overflow = overflow_.load(std::memory_order_relaxed);
    result.samples += result.overflow;
    if (!result.samples) return result;

    const uint64_t rank = (result.samples * 999ULL + 999ULL) / 1000ULL;
    uint64_t cumulative = 0;
    for (size_t i = 0; i < counts.size(); ++i) {
      cumulative += counts[i];
      if (cumulative >= rank) {
        result.p999_permille_upper = i;
        return result;
      }
    }
    result.p999_permille_upper = kMaxPermille + 1;
    result.p999_overflow = true;
    return result;
  }

 private:
  std::array<std::atomic<uint64_t>, kMaxPermille + 1> bins_{};
  std::atomic<uint64_t> overflow_{0};
};

}  // namespace tmp_nam
