#include "../nam_loader.h"
#include "../nam_registry.h"

#include <cassert>
#include <chrono>
#include <iostream>
#include <stdexcept>
#include <vector>

namespace {
using Loader = tmp_nam::LatestLoader<int, 3>;
using Registry = tmp_nam::Registry<int, 3>;

class Gate {
 public:
  void arrive_and_wait() {
    std::unique_lock<std::mutex> lock(mutex_);
    entered_ = true;
    changed_.notify_all();
    assert(changed_.wait_for(lock, std::chrono::seconds(5), [this] { return released_; }));
  }
  void wait_until_entered() {
    std::unique_lock<std::mutex> lock(mutex_);
    assert(changed_.wait_for(lock, std::chrono::seconds(5), [this] { return entered_; }));
  }
  void release() {
    std::lock_guard<std::mutex> lock(mutex_);
    released_ = true;
    changed_.notify_all();
  }
 private:
  std::mutex mutex_;
  std::condition_variable changed_;
  bool entered_ = false, released_ = false;
};

class Completions {
 public:
  void add() {
    std::lock_guard<std::mutex> lock(mutex_);
    ++count_;
    changed_.notify_all();
  }
  void wait_for(unsigned count) {
    std::unique_lock<std::mutex> lock(mutex_);
    assert(changed_.wait_for(lock, std::chrono::seconds(5), [this, count] { return count_ >= count; }));
  }
 private:
  std::mutex mutex_;
  std::condition_variable changed_;
  unsigned count_ = 0;
};

void test_coalescing_and_cancellation() {
  Gate first;
  Completions completed;
  std::vector<int> executed;
  std::vector<std::thread::id> threads;
  {
    Loader loader([&](Loader::Request request) {
      if (request.payload == 0) first.arrive_and_wait();
      executed.push_back(request.payload);
      threads.push_back(std::this_thread::get_id());
      completed.add();
    });
    assert(loader.submit(0, 1, 0));
    first.wait_until_entered();
    for (int generation = 1; generation <= 100; ++generation)
      assert(loader.submit(1, generation, generation));
    assert(loader.submit(2, 1, 201));
    assert(loader.outstanding() == 3);  // active + one pending per other slot
    loader.cancel(2, 1);
    assert(loader.outstanding() == 2);
    assert(!loader.submit(2, 1, 201));  // delayed enqueue after cancellation
    assert(loader.submit(2, 2, 202));
    loader.cancel(2, 1);              // an old cancel cannot remove its successor
    assert(!loader.submit(1, 99, 99));
    first.release();
    completed.wait_for(3);
  }
  assert((executed == std::vector<int>{0, 100, 202}));
  assert(threads.size() == 3 && threads[0] == threads[1] && threads[1] == threads[2]);
  assert(threads[0] != std::this_thread::get_id());
}

void test_registry_checks_before_load_and_publication() {
  Registry registry;
  int instance;
  Gate before_load, before_publish;
  Completions completed;
  unsigned expensive_loads = 0, skipped = 0, stale_publications = 0;
  {
    Loader loader([&](Loader::Request request) {
      const Registry::Ticket ticket{request.slot, request.generation, false};
      if (request.payload == 1) before_load.arrive_and_wait();
      if (!registry.current(ticket)) {
        ++skipped;
      } else {
        ++expensive_loads;
        auto player = std::make_shared<int>(request.payload);
        if (request.payload == 2) before_publish.arrive_and_wait();
        if (!registry.publish(ticket, std::move(player))) ++stale_publications;
      }
      completed.add();
    });
    auto first = registry.begin(&instance);
    assert(loader.submit(first.index, first.generation, 1));
    before_load.wait_until_entered();
    const auto cleared = registry.clear_details(&instance);
    assert(cleared.cleared);
    loader.cancel(cleared.index, cleared.generation);
    // An ordinary IR or destructor cancels the active request too. Reusing the
    // same instance address must not revive the cancelled generation.
    auto second = registry.begin(&instance);
    assert(loader.submit(second.index, second.generation, 2));
    before_load.release();
    before_publish.wait_until_entered();
    auto third = registry.begin(&instance);
    assert(loader.submit(third.index, third.generation, 3));
    before_publish.release();
    completed.wait_for(3);
  }
  assert(skipped == 1 && expensive_loads == 2 && stale_publications == 1);
  assert(*registry.read(&instance).get() == 3);
}

void test_failure_preserves_ready_player_and_worker() {
  Registry registry;
  int instance;
  auto initial = registry.begin(&instance);
  assert(registry.publish(initial, std::make_shared<int>(7)));
  Gate failed, replacement;
  Completions completed;
  {
    Loader loader([&](Loader::Request request) {
      if (request.payload == 1) {
        failed.arrive_and_wait();
        throw std::runtime_error("invalid model");
      }
      replacement.arrive_and_wait();
      assert(registry.publish({request.slot, request.generation, true}, std::make_shared<int>(9)));
      completed.add();
    });
    auto invalid = registry.begin(&instance);
    assert(invalid.had_ready);
    assert(loader.submit(invalid.index, invalid.generation, 1));
    failed.wait_until_entered();
    auto next = registry.begin(&instance);
    assert(loader.submit(next.index, next.generation, 2));
    failed.release();
    replacement.wait_until_entered();
    assert(loader.errors() == 1);
    assert(*registry.read(&instance).get() == 7);
    replacement.release();
    completed.wait_for(1);
  }
  assert(*registry.read(&instance).get() == 9);
}
}  // namespace

int main() {
  test_coalescing_and_cancellation();
  test_registry_checks_before_load_and_publication();
  test_failure_preserves_ready_player_and_worker();
  std::cout << "NAM bounded loader coalescing, cancellation, and stale generations: PASS\n";
}
