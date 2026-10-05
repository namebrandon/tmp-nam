#pragma once

#include <array>
#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <vector>

namespace tmp_nam {

// Fixed slots keep reader counters alive even after an IRProcessor is destroyed.
// Control operations serialize; audio readers never acquire the control mutex.
// Sequential consistency orders reader admission, pointer publication and the
// grace-period check. Do not weaken these orders independently.
template <class T, size_t Capacity = 64> class Registry {
  struct Slot {
    std::atomic<void*> key{nullptr};
    std::atomic<T*> current{nullptr};
    std::atomic<unsigned> readers{0};
    uint64_t generation = 0;  // control mutex only
    std::shared_ptr<T> owner;
    std::vector<std::shared_ptr<T>> retired;
  };
  std::array<Slot, Capacity> slots_;
  std::mutex control_;

  static void replace(Slot& slot, std::shared_ptr<T> next) {
    // Retire ownership BEFORE exposing the replacement. Allocation is on the
    // control thread and cannot strand a published pointer on an exception.
    if (slot.owner) slot.retired.push_back(slot.owner);
    slot.owner = std::move(next);
    slot.current.store(slot.owner.get());
  }

 public:
  struct Ticket {
    size_t index;
    uint64_t generation;
    bool had_ready;
  };
  struct ClearResult {
    bool cleared = false;
    size_t index = Capacity;
    uint64_t generation = 0;
  };
  class Read {
    Slot* slot_ = nullptr;
    T* value_ = nullptr;
    friend class Registry;
    Read(Slot* slot, T* value) : slot_(slot), value_(value) {}
   public:
    Read() = default;
    Read(const Read&) = delete;
    Read& operator=(const Read&) = delete;
    Read(Read&& other) noexcept : slot_(other.slot_), value_(other.value_) {
      other.slot_ = nullptr;
    }
    ~Read() { if (slot_) slot_->readers.fetch_sub(1); }
    bool bound() const { return slot_ != nullptr; }
    T* get() const { return value_; }
  };

  // Begin a replacement without discarding the last ready player. Each load has
  // a generation, so a superseded attempt cannot publish over a newer binding.
  Ticket begin(void* key) {
    if (!key) throw std::invalid_argument("null NAM instance");
    std::lock_guard<std::mutex> lock(control_);
    size_t empty = Capacity;
    for (size_t i = 0; i < Capacity; ++i) {
      void* existing = slots_[i].key.load();
      if (existing == key)
        return {i, ++slots_[i].generation, slots_[i].owner != nullptr};
      if (!existing && empty == Capacity) empty = i;
    }
    if (empty == Capacity) throw std::runtime_error("NAM instance capacity exhausted");
    Slot& slot = slots_[empty];
    ++slot.generation;
    slot.key.store(key);
    return {empty, slot.generation, false};
  }

  bool publish(Ticket ticket, std::shared_ptr<T> player) {
    std::lock_guard<std::mutex> lock(control_);
    Slot& slot = slots_.at(ticket.index);
    if (!slot.key.load() || slot.generation != ticket.generation) return false;
    replace(slot, std::move(player));
    return true;
  }

  bool current(Ticket ticket) {
    std::lock_guard<std::mutex> lock(control_);
    const Slot& slot = slots_.at(ticket.index);
    return slot.key.load() && slot.generation == ticket.generation;
  }

  // Abandon only the exact unpublished generation. A newer generation is
  // never disturbed. An existing ready owner remains available; an empty slot
  // is released so a failed first load does not consume registry capacity.
  bool cancel(Ticket ticket) {
    std::lock_guard<std::mutex> lock(control_);
    Slot& slot = slots_.at(ticket.index);
    if (!slot.key.load() || slot.generation != ticket.generation) return false;
    ++slot.generation;
    if (!slot.owner) {
      slot.current.store(nullptr);
      slot.key.store(nullptr);
    }
    return true;
  }

  ClearResult clear_details(void* key) {
    std::lock_guard<std::mutex> lock(control_);
    for (size_t index = 0; index < Capacity; ++index) {
      auto& slot = slots_[index];
      if (slot.key.load() == key && key) {
        const uint64_t cancelled_generation = slot.generation;
        replace(slot, {});
        slot.key.store(nullptr);
        ++slot.generation;
        return {true, index, cancelled_generation};
      }
    }
    return {};
  }

  bool clear(void* key) { return clear_details(key).cleared; }

  Read read(void* key) noexcept {
    if (!key) return {};
    for (auto& slot : slots_) {
      if (slot.key.load() != key) continue;
      slot.readers.fetch_add(1);
      if (slot.key.load() == key) return Read(&slot, slot.current.load());
      slot.readers.fetch_sub(1);
    }
    return {};
  }

  // Worker-only: reclaim after a grace period and take stable references for
  // telemetry. Destructors therefore never run on an audio reader.
  std::vector<std::shared_ptr<T>> collect() {
    std::lock_guard<std::mutex> lock(control_);
    std::vector<std::shared_ptr<T>> active;
    for (auto& slot : slots_) {
      if (slot.readers.load() == 0) slot.retired.clear();
      if (slot.owner) active.push_back(slot.owner);
    }
    return active;
  }
};
}  // namespace tmp_nam
