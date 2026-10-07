#include "../nam_registry.h"
#include <cassert>
#include <iostream>
#include <thread>

struct State {
  int value;
  std::atomic<int>* destroyed;
  ~State() { destroyed->fetch_add(1); }
};

int main() {
  std::atomic<int> destroyed{0};
  tmp_nam::Registry<State, 3> registry;
  int a, b;
  auto make = [&](int value) {
    return std::shared_ptr<State>(new State{value, &destroyed});
  };
  auto first = registry.begin(&a);
  assert(!first.had_ready);
  assert(registry.read(&a).bound());
  assert(!registry.read(&a).get());
  assert(registry.publish(first, make(1)));
  auto first_clear = registry.clear_details(&a);
  assert(first_clear.cleared && first_clear.index == first.index &&
         first_clear.generation == first.generation);
  assert(!registry.clear_details(&a).cleared);
  registry.collect();
  assert(destroyed.load() == 1);
  first = registry.begin(&a);
  assert(registry.publish(first, make(1)));
  auto second = registry.begin(&a);
  assert(second.had_ready);
  assert(registry.read(&a).get()->value == 1);  // ready state survives loading
  assert(registry.cancel(second));
  assert(registry.read(&a).get()->value == 1);  // cancel retains last ready
  assert(!registry.publish(second, make(98)));  // cancelled ticket is stale
  second = registry.begin(&a);
  assert(second.had_ready);
  assert(!registry.publish(first, make(99)));  // stale load loses
  const int already_destroyed = destroyed.load();
  {
    auto reader = registry.read(&a);
    assert(registry.publish(second, make(2)));
    registry.collect();
    assert(reader.get()->value == 1);
    assert(destroyed.load() == already_destroyed);
  }
  registry.collect();
  assert(destroyed.load() == already_destroyed + 1);
  auto stale = registry.begin(&a);
  assert(registry.clear(&a));
  assert(!registry.clear(&a));
  assert(!registry.read(&a).bound());
  assert(!registry.publish(stale, make(3)));
  auto reused = registry.begin(&a);
  assert(!reused.had_ready);
  assert(registry.publish(reused, make(4)));
  auto distinct = registry.begin(&b);
  assert(registry.publish(distinct, make(5)));
  assert(registry.read(&a).get() != registry.read(&b).get());

  // clear_if_current drops a ready owner for its own generation only.
  int failed_key;
  auto failed_first = registry.begin(&failed_key);
  assert(registry.publish(failed_first, make(7)));
  auto failed = registry.begin(&failed_key);
  auto newer = registry.begin(&failed_key);
  assert(!registry.clear_if_current(failed));  // a newer load is never disturbed
  assert(registry.read(&failed_key).get()->value == 7);
  assert(registry.clear_if_current(newer));
  assert(!registry.read(&failed_key).bound());
  assert(!registry.publish(newer, make(8)));

  int empty_key;
  auto empty = registry.begin(&empty_key);
  assert(!empty.had_ready);
  assert(registry.cancel(empty));
  assert(!registry.read(&empty_key).bound());
  assert(!registry.publish(empty, make(6)));

  std::atomic<bool> stop{false};
  std::thread audio([&] {
    while (!stop.load()) {
      auto state = registry.read(&a);
      if (state.get()) assert(state.get()->value >= 4);
    }
  });
  for (int i = 0; i < 10000; ++i) {
    auto request = registry.begin(&a);
    registry.publish(request, make(i + 4));
    if (i % 7 == 0) registry.clear(&a);
    registry.collect();
  }
  stop.store(true);
  audio.join();
  registry.clear(&a);
  registry.clear(&b);
  registry.collect();
  std::cout << "NAM registry lifecycle/concurrent publication: PASS\n";
}
