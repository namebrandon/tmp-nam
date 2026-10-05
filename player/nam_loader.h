#pragma once

#include <array>
#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <functional>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <thread>
#include <utility>

namespace tmp_nam {

// One control-side worker, with at most one pending request per registry slot.
// Generations also reject a late submit after replacement or cancellation.
// None of these operations belongs on the audio callback.
template <class Payload, size_t Capacity = 64> class LatestLoader {
  static_assert(Capacity > 0, "NAM loader needs at least one slot");

 public:
  struct Request {
    size_t slot;
    uint64_t generation;
    Payload payload;
  };

  explicit LatestLoader(std::function<void(Request)> work)
      : work_(std::move(work)), worker_([this] { run(); }) {}

  ~LatestLoader() {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      stopping_ = true;
      for (auto& request : pending_) {
        if (request) outstanding_.fetch_sub(1);
        request.reset();
      }
    }
    ready_.notify_one();
    worker_.join();
  }

  LatestLoader(const LatestLoader&) = delete;
  LatestLoader& operator=(const LatestLoader&) = delete;

  bool submit(size_t slot, uint64_t generation, Payload payload) {
    if (slot >= Capacity) throw std::out_of_range("NAM loader slot");
    {
      std::lock_guard<std::mutex> lock(mutex_);
      if (stopping_ || generation <= latest_[slot]) return false;
      const bool was_pending = pending_[slot].has_value();
      pending_[slot] = Request{slot, generation, std::move(payload)};
      latest_[slot] = generation;
      if (!was_pending) outstanding_.fetch_add(1);
    }
    ready_.notify_one();
    return true;
  }

  void cancel(size_t slot, uint64_t generation) {
    if (slot >= Capacity) return;
    std::lock_guard<std::mutex> lock(mutex_);
    if (generation > latest_[slot]) latest_[slot] = generation;
    if (pending_[slot] && pending_[slot]->generation <= generation) {
      pending_[slot].reset();
      outstanding_.fetch_sub(1);
    }
    // An active request checks the registry again before publishing its result.
  }

  size_t outstanding() const noexcept { return outstanding_.load(); }
  uint64_t errors() const noexcept { return errors_.load(); }

 private:
  bool has_pending() const {
    for (const auto& request : pending_)
      if (request) return true;
    return false;
  }

  void run() {
    for (;;) {
      std::optional<Request> request;
      {
        std::unique_lock<std::mutex> lock(mutex_);
        ready_.wait(lock, [this] { return stopping_ || has_pending(); });
        if (stopping_) return;
        for (size_t i = 0; i < Capacity; ++i) {
          const size_t slot = (next_slot_ + i) % Capacity;
          if (!pending_[slot]) continue;
          request = std::move(pending_[slot]);
          pending_[slot].reset();
          next_slot_ = (slot + 1) % Capacity;
          break;
        }
      }
      try {
        work_(std::move(*request));
      } catch (...) {
        // Keep the worker available if an unexpected control-side error escapes.
        // The dispatcher reports this counter from its telemetry worker.
        errors_.fetch_add(1);
      }
      {
        std::lock_guard<std::mutex> lock(mutex_);
        outstanding_.fetch_sub(1);
      }
    }
  }

  std::function<void(Request)> work_;
  mutable std::mutex mutex_;
  std::condition_variable ready_;
  std::array<std::optional<Request>, Capacity> pending_;
  std::array<uint64_t, Capacity> latest_{};
  size_t next_slot_ = 0;
  bool stopping_ = false;
  std::atomic<size_t> outstanding_{0};
  std::atomic<uint64_t> errors_{0};
  std::thread worker_;
};

}  // namespace tmp_nam
