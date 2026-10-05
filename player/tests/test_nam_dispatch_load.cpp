#define TMP_NAM_DISPATCH_TEST 1
#include "../stubs/nam_dispatch.cpp"

#include <atomic>
#include <cassert>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <type_traits>
#include <vector>

namespace {

namespace fs = std::filesystem;
using Registry = tmp_nam::Registry<NamEntry, kInstanceCapacity>;

struct OriginalLoadFailure {};

thread_local loadfile_result_t g_original_result = 0;
thread_local bool g_original_throws = false;
thread_local std::string g_redirected_path;
std::atomic<unsigned> g_original_calls{0};

loadfile_result_t fake_original_loadfile(void*, void* str_ref) {
  const auto& path = *static_cast<const std::string*>(str_ref);
  g_original_calls.fetch_add(1);
  g_redirected_path = classify_and_redirect(path.c_str());
  if (g_original_throws) throw OriginalLoadFailure{};
  return g_original_result;
}

loadfile_result_t invoke(void* self, const std::string& path,
                         loadfile_result_t result, bool throws = false) {
  g_original_result = result;
  g_original_throws = throws;
  g_redirected_path.clear();
  return loadfile_handler(self, const_cast<std::string*>(&path));
}

template <class Fn> void expect_runtime_error(Fn&& fn) {
  bool caught = false;
  try {
    fn();
  } catch (const std::runtime_error&) {
    caught = true;
  }
  assert(caught);
}

class Gate {
 public:
  void arrive_and_wait() {
    std::unique_lock<std::mutex> lock(mutex_);
    entered_ = true;
    changed_.notify_all();
    assert(changed_.wait_for(lock, std::chrono::seconds(10),
                             [this] { return released_; }));
  }

  void wait_until_entered() {
    std::unique_lock<std::mutex> lock(mutex_);
    assert(changed_.wait_for(lock, std::chrono::seconds(10),
                             [this] { return entered_; }));
  }

  void release() {
    std::lock_guard<std::mutex> lock(mutex_);
    released_ = true;
    changed_.notify_all();
  }

 private:
  std::mutex mutex_;
  std::condition_variable changed_;
  bool entered_ = false;
  bool released_ = false;
};

class SubmissionTracker {
 public:
  void record(void* self, Registry::Ticket ticket) {
    std::lock_guard<std::mutex> lock(mutex_);
    submissions_.push_back({self, ticket});
    changed_.notify_all();
  }

  void wait_for(std::size_t count) {
    std::unique_lock<std::mutex> lock(mutex_);
    assert(changed_.wait_for(lock, std::chrono::seconds(10),
                             [this, count] {
                               return submissions_.size() >= count;
                             }));
  }

 private:
  std::mutex mutex_;
  std::condition_variable changed_;
  std::vector<std::pair<void*, Registry::Ticket>> submissions_;
};

Gate* g_active_prepare_gate = nullptr;
SubmissionTracker* g_submission_tracker = nullptr;
std::atomic<unsigned> g_prepare_gate_calls{0};

void wait_in_prepare() {
  assert(g_active_prepare_gate);
  g_active_prepare_gate->arrive_and_wait();
}

void wait_in_first_prepare() {
  assert(g_active_prepare_gate);
  if (g_prepare_gate_calls.fetch_add(1) == 0)
    g_active_prepare_gate->arrive_and_wait();
}

void record_submission(void* self, Registry::Ticket ticket) {
  assert(g_submission_tracker);
  g_submission_tracker->record(self, ticket);
}

bool is_runtime_error(const std::exception_ptr& error) {
  if (!error) return false;
  try {
    std::rethrow_exception(error);
  } catch (const std::runtime_error&) {
    return true;
  } catch (...) {
    return false;
  }
}

std::shared_ptr<NamEntry> g_injected_newer;

void publish_newer_generation(void* self, Registry::Ticket) {
  assert(g_injected_newer);
  const auto newer = g_players.begin(self);
  g_injected_newer->slot = newer.index;
  g_injected_newer->generation = newer.generation;
  assert(g_players.publish(newer, g_injected_newer));
}

void bind_fresh(void* self, const std::string& path) {
  auto entry = prepare_instance(path);
  const auto ticket = g_players.begin(self);
  entry->slot = ticket.index;
  entry->generation = ticket.generation;
  assert(g_players.publish(ticket, std::move(entry)));
}

NamEntry* bound_entry(void* self) {
  auto state = g_players.read(self);
  assert(state.bound());
  assert(state.get());
  return state.get();
}

void clear_and_collect(void* self) {
  (void)g_players.clear(self);
  (void)g_players.collect();
}

void test_disarmed_stock_path(const std::string& nam_path) {
  int instance = 0;
  g_armed.store(false, std::memory_order_release);
  const std::string ordinary = "/tmp/ordinary.wav";
  for (const auto value : {loadfile_result_t{0}, loadfile_result_t{1},
                           loadfile_result_t{0xa5}}) {
    g_original_calls.store(0);
    assert(invoke(&instance, ordinary, value) == value);
    assert(g_original_calls.load() == 1);
    assert(g_redirected_path == ordinary);
    assert(invoke(&instance, nam_path, value) == value);
    assert(g_original_calls.load() == 2);
    assert(g_redirected_path == nam_path);
  }
  assert(!g_players.read(&instance).bound());
}

void test_ordinary_compatibility_and_exceptions(
    const std::string& valid_nam) {
  int exact_instance = 0;
  int clear_instance = 0;
  int ordinary_throw_instance = 0;
  int nam_throw_instance = 0;
  const std::string ordinary = "/tmp/ordinary.wav";
  g_armed.store(true, std::memory_order_release);

  for (const auto value : {loadfile_result_t{0}, loadfile_result_t{1},
                           loadfile_result_t{0xa5}}) {
    g_original_calls.store(0);
    assert(invoke(&exact_instance, ordinary, value) == value);
    assert(g_original_calls.load() == 1);
    assert(g_redirected_path == ordinary);
  }

  bind_fresh(&clear_instance, valid_nam);
  assert(invoke(&clear_instance, ordinary, loadfile_result_t{0xa5}) == 0xa5);
  assert(!g_players.read(&clear_instance).bound());

  bind_fresh(&ordinary_throw_instance, valid_nam);
  bool caught = false;
  try {
    (void)invoke(&ordinary_throw_instance, ordinary, 1, true);
  } catch (const OriginalLoadFailure&) {
    caught = true;
  }
  assert(caught);
  assert(!g_players.read(&ordinary_throw_instance).bound());

  bind_fresh(&nam_throw_instance, valid_nam);
  NamEntry* previous = bound_entry(&nam_throw_instance);
  caught = false;
  try {
    (void)invoke(&nam_throw_instance, valid_nam, 1, true);
  } catch (const OriginalLoadFailure&) {
    caught = true;
  }
  assert(caught);
  assert(bound_entry(&nam_throw_instance) == previous);
  g_original_throws = false;

  clear_and_collect(&ordinary_throw_instance);
  clear_and_collect(&nam_throw_instance);
}

void test_recognized_nam_failures(const std::string& valid_nam,
                                  const std::string& invalid_nam) {
  int disabled_instance = 0;
  int stock_failure_instance = 0;
  int invalid_instance = 0;
  int retained_instance = 0;

  g_process_hook_enabled = false;
  g_armed.store(true, std::memory_order_release);
  g_original_calls.store(0);
  expect_runtime_error([&] {
    (void)invoke(&disabled_instance, valid_nam, loadfile_result_t{0xa5});
  });
  assert(g_original_calls.load() == 1);
  assert(g_redirected_path == valid_nam);
  assert(!g_armed.load(std::memory_order_acquire));
  assert(!g_players.read(&disabled_instance).bound());

  g_process_hook_enabled = true;
  g_armed.store(true, std::memory_order_release);
  expect_runtime_error([&] {
    (void)invoke(&stock_failure_instance, valid_nam, 0);
  });
  assert(g_redirected_path == g_stub_wav_path);
  assert(!g_players.read(&stock_failure_instance).bound());

  expect_runtime_error([&] {
    (void)invoke(&invalid_instance, invalid_nam, 1);
  });
  assert(g_redirected_path == g_stub_wav_path);
  assert(!g_players.read(&invalid_instance).bound());

  bind_fresh(&retained_instance, valid_nam);
  NamEntry* previous = bound_entry(&retained_instance);
  expect_runtime_error([&] {
    (void)invoke(&retained_instance, invalid_nam, 1);
  });
  assert(bound_entry(&retained_instance) == previous);
  clear_and_collect(&retained_instance);
  assert(g_live_entries.load() == 0);
}

void test_delayed_first_load(const std::string& path) {
  int instance = 0;
  Gate gate;
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_prepare;
  std::atomic<bool> completed{false};
  std::exception_ptr failure;
  std::thread control([&] {
    try {
      assert(invoke(&instance, path, 1) == 1);
    } catch (...) {
      failure = std::current_exception();
    }
    completed.store(true, std::memory_order_release);
  });
  gate.wait_until_entered();
  assert(!completed.load(std::memory_order_acquire));
  {
    auto pending = g_players.read(&instance);
    assert(pending.bound() && !pending.get());
  }
  gate.release();
  control.join();
  g_prepare_gate = nullptr;
  g_active_prepare_gate = nullptr;
  assert(!failure);
  assert(bound_entry(&instance)->path == path);
  clear_and_collect(&instance);
  assert(g_live_entries.load() == 0);
}

void test_synchronous_ready_and_reader_grace(
    const std::string& first_nam, const std::string& second_nam) {
  int instance = 0;
  bind_fresh(&instance, first_nam);
  const std::size_t baseline = g_live_entries.load();
  assert(baseline == 1);

  Gate gate;
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_prepare;
  std::atomic<bool> completed{false};
  loadfile_result_t result = 0;
  std::exception_ptr failure;

  {
    auto old_reader = g_players.read(&instance);
    assert(old_reader.bound() && old_reader.get());
    NamEntry* old_entry = old_reader.get();
    tmp_nam::Player* old_player = old_entry->player.get();
    std::thread control([&] {
      try {
        result = invoke(&instance, second_nam, loadfile_result_t{0xa5});
      } catch (...) {
        failure = std::current_exception();
      }
      completed.store(true, std::memory_order_release);
    });

    gate.wait_until_entered();
    assert(!completed.load(std::memory_order_acquire));
    assert(bound_entry(&instance) == old_entry);

    std::vector<float> input(32, 0.1f);
    std::vector<float> output(32, 0.0f);
    process_handler(&instance, input.data(), output.data(),
                    reinterpret_cast<void*>(uintptr_t{32}), nullptr, nullptr,
                    nullptr, nullptr);
    assert(old_entry->player.get() == old_player);
    assert(old_entry->blocks.load() == 1);
    for (float sample : output) assert(std::isfinite(sample));

    gate.release();
    control.join();
    g_prepare_gate = nullptr;
    g_active_prepare_gate = nullptr;
    assert(!failure);
    assert(result == 0xa5);
    assert(completed.load(std::memory_order_acquire));
    NamEntry* ready = bound_entry(&instance);
    assert(ready != old_entry);
    assert(ready->player.get() != old_player);
    assert(ready->path == second_nam);
    assert(ready->test_worker_thread != ready->test_control_thread);

    // Collection cannot destroy the old owner while a callback reader pins it.
    (void)g_players.collect();
    assert(g_live_entries.load() == baseline + 1);
    assert(old_reader.get() == old_entry);
  }

  // Reader release allows control-side collection. Repeated collection cannot
  // autonomously replace the completed synchronous binding.
  NamEntry* completed_entry = bound_entry(&instance);
  (void)g_players.collect();
  assert(g_live_entries.load() == 1);
  assert(bound_entry(&instance) == completed_entry);
  (void)g_players.collect();
  assert(bound_entry(&instance) == completed_entry);
  clear_and_collect(&instance);
  assert(g_live_entries.load() == 0);
}

void test_rapid_abc_supersession(
    const std::string& older_path, const std::string& middle_path,
    const std::string& newer_path) {
  int instance = 0;
  Gate gate;
  SubmissionTracker submissions;
  g_prepare_gate_calls.store(0);
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_first_prepare;
  g_submission_tracker = &submissions;
  g_after_submit = record_submission;
  bool older_succeeded = false;
  bool middle_succeeded = false;
  bool newer_succeeded = false;
  std::exception_ptr older_error;
  std::exception_ptr middle_error;
  std::exception_ptr newer_error;

  std::thread older([&] {
    try {
      older_succeeded = invoke(&instance, older_path, 1) != 0;
    } catch (...) {
      older_error = std::current_exception();
    }
  });
  gate.wait_until_entered();
  std::thread middle([&] {
    try {
      middle_succeeded = invoke(&instance, middle_path, 1) != 0;
    } catch (...) {
      middle_error = std::current_exception();
    }
  });
  submissions.wait_for(2);
  std::thread newer([&] {
    try {
      newer_succeeded = invoke(&instance, newer_path, 1) != 0;
    } catch (...) {
      newer_error = std::current_exception();
    }
  });
  submissions.wait_for(3);
  // C coalesces the queued B; B must finish with failure before A is released.
  middle.join();
  assert(!middle_succeeded);
  assert(is_runtime_error(middle_error));
  gate.release();
  older.join();
  newer.join();
  g_prepare_gate = nullptr;
  g_active_prepare_gate = nullptr;
  g_after_submit = nullptr;
  g_submission_tracker = nullptr;

  assert(!older_succeeded);
  assert(is_runtime_error(older_error));
  assert(newer_succeeded);
  assert(!newer_error);
  assert(bound_entry(&instance)->path == newer_path);
  clear_and_collect(&instance);
  assert(g_live_entries.load() == 0);
}

void test_ordinary_clear_cannot_be_resurrected(
    const std::string& older_path) {
  int instance = 0;
  const std::string ordinary = "/tmp/ordinary.wav";
  Gate gate;
  g_prepare_gate_calls.store(0);
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_first_prepare;
  bool older_succeeded = false;
  std::exception_ptr older_error;

  std::thread older([&] {
    try {
      older_succeeded = invoke(&instance, older_path, 1) != 0;
    } catch (...) {
      older_error = std::current_exception();
    }
  });
  gate.wait_until_entered();
  assert(invoke(&instance, ordinary, loadfile_result_t{0xa5}) == 0xa5);
  assert(!g_players.read(&instance).bound());
  gate.release();
  older.join();
  g_prepare_gate = nullptr;
  g_active_prepare_gate = nullptr;

  assert(!older_succeeded);
  assert(is_runtime_error(older_error));
  assert(!g_players.read(&instance).bound());
  (void)g_players.collect();
  assert(g_live_entries.load() == 0);
}

void test_queued_cancellation_releases_waiter_and_worker_recovers(
    const std::string& held_path, const std::string& queued_path,
    const std::string& recovery_path) {
  int held_instance = 0;
  int queued_instance = 0;
  const std::string ordinary = "/tmp/ordinary.wav";
  Gate gate;
  SubmissionTracker submissions;
  g_prepare_gate_calls.store(0);
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_first_prepare;
  g_submission_tracker = &submissions;
  g_after_submit = record_submission;

  bool held_succeeded = false;
  bool queued_succeeded = false;
  std::exception_ptr held_error;
  std::exception_ptr queued_error;
  std::thread held([&] {
    try {
      held_succeeded = invoke(&held_instance, held_path, 1) != 0;
    } catch (...) {
      held_error = std::current_exception();
    }
  });
  gate.wait_until_entered();

  std::thread queued([&] {
    try {
      queued_succeeded = invoke(&queued_instance, queued_path, 1) != 0;
    } catch (...) {
      queued_error = std::current_exception();
    }
  });
  submissions.wait_for(2);

  assert(invoke(&queued_instance, ordinary, loadfile_result_t{0xa5}) == 0xa5);
  queued.join();
  assert(!queued_succeeded);
  assert(is_runtime_error(queued_error));
  assert(!g_players.read(&queued_instance).bound());

  gate.release();
  held.join();
  g_prepare_gate = nullptr;
  g_active_prepare_gate = nullptr;
  g_after_submit = nullptr;
  g_submission_tracker = nullptr;
  assert(held_succeeded);
  assert(!held_error);

  // Destroying the queued packaged task produced a broken-promise failure for
  // its waiter; the single existing worker remains available afterwards.
  assert(invoke(&queued_instance, recovery_path, 1) == 1);
  assert(bound_entry(&queued_instance)->path == recovery_path);
  clear_and_collect(&held_instance);
  clear_and_collect(&queued_instance);
  assert(g_live_entries.load() == 0);
}

void test_preparation_exception_leaves_worker_usable(
    const std::string& invalid_path, const std::string& valid_path) {
  int instance = 0;
  expect_runtime_error([&] { (void)invoke(&instance, invalid_path, 1); });
  assert(!g_players.read(&instance).bound());
  assert(invoke(&instance, valid_path, 1) == 1);
  assert(bound_entry(&instance)->path == valid_path);
  clear_and_collect(&instance);
  assert(g_live_entries.load() == 0);
}

void test_held_invalid_cleanup_preserves_newer(
    const std::string& invalid_path, const std::string& newer_path) {
  int instance = 0;
  Gate gate;
  SubmissionTracker submissions;
  g_prepare_gate_calls.store(0);
  g_active_prepare_gate = &gate;
  g_prepare_gate = wait_in_first_prepare;
  g_submission_tracker = &submissions;
  g_after_submit = record_submission;
  bool older_succeeded = false;
  bool newer_succeeded = false;
  std::exception_ptr older_error;
  std::exception_ptr newer_error;

  std::thread older([&] {
    try {
      older_succeeded = invoke(&instance, invalid_path, 1) != 0;
    } catch (...) {
      older_error = std::current_exception();
    }
  });
  gate.wait_until_entered();
  std::thread newer([&] {
    try {
      newer_succeeded = invoke(&instance, newer_path, 1) != 0;
    } catch (...) {
      newer_error = std::current_exception();
    }
  });
  submissions.wait_for(2);
  gate.release();
  older.join();
  newer.join();
  g_prepare_gate = nullptr;
  g_active_prepare_gate = nullptr;
  g_after_submit = nullptr;
  g_submission_tracker = nullptr;

  assert(!older_succeeded);
  assert(is_runtime_error(older_error));
  assert(newer_succeeded);
  assert(!newer_error);
  assert(bound_entry(&instance)->path == newer_path);
  clear_and_collect(&instance);
  assert(g_live_entries.load() == 0);
}

void test_fresh_sequences_and_identical_captures(
    const std::string& a, const std::string& b, const std::string& c) {
  int sequence = 0;
  int first = 0;
  int second = 0;
  g_armed.store(true, std::memory_order_release);

  assert(invoke(&sequence, a, 1) == 1);
  auto* a1 = bound_entry(&sequence)->player.get();
  assert(invoke(&sequence, b, 1) == 1);
  auto* b1 = bound_entry(&sequence)->player.get();
  assert(b1 != a1);
  assert(invoke(&sequence, c, 1) == 1);
  auto* c1 = bound_entry(&sequence)->player.get();
  assert(c1 != a1 && c1 != b1);
  assert(bound_entry(&sequence)->path == c);
  assert(invoke(&sequence, a, 1) == 1);
  auto* a2 = bound_entry(&sequence)->player.get();
  assert(a2 != a1 && a2 != b1 && a2 != c1);
  assert(bound_entry(&sequence)->path == a);

  assert(invoke(&first, a, 1) == 1);
  assert(invoke(&second, a, 1) == 1);
  assert(bound_entry(&first)->player.get() !=
         bound_entry(&second)->player.get());

  clear_and_collect(&sequence);
  clear_and_collect(&first);
  clear_and_collect(&second);
  assert(g_live_entries.load() == 0);
}

void test_stale_publication_cannot_erase_newer(
    const std::string& candidate, const std::string& newer_path) {
  int instance = 0;
  g_injected_newer = prepare_instance(newer_path);
  g_before_publish = publish_newer_generation;
  expect_runtime_error([&] { (void)invoke(&instance, candidate, 1); });
  g_before_publish = nullptr;
  assert(bound_entry(&instance) == g_injected_newer.get());
  assert(bound_entry(&instance)->path == newer_path);
  clear_and_collect(&instance);
  g_injected_newer.reset();
  (void)g_players.collect();
  assert(g_live_entries.load() == 0);
}

}  // namespace

static_assert(
    std::is_same<decltype(loadfile_handler(nullptr, nullptr)),
                 std::uint8_t>::value,
    "IRProcessor::loadFile hook must preserve the firmware's low return byte");

int main(int argc, char** argv) {
  assert(argc == 2);
  const fs::path fixture = argv[1];
  assert(fs::is_regular_file(fixture));

  const fs::path work = fs::temp_directory_path() /
      ("tmp-nam-dispatch-load-" + std::to_string(getpid()));
  fs::remove_all(work);
  fs::create_directories(work);
  const fs::path valid_a = work / "a.nam.wav";
  const fs::path valid_b = work / "b.nam.wav";
  const fs::path valid_c = work / "c.nam.wav";
  const fs::path invalid = work / "invalid.nam.wav";
  const fs::path stub_wav = work / "stub.wav";
  fs::copy_file(fixture, valid_a);
  fs::copy_file(fixture, valid_b);
  fs::copy_file(fixture, valid_c);
  {
    std::ofstream bad(invalid);
    bad << "not a NAM model\n";
  }

  const std::string stub_wav_string = stub_wav.string();
  g_stub_wav_path = stub_wav_string.c_str();
  assert(ensure_stub_wav());
  g_loadfile_tramp.exec_buffer = reinterpret_cast<void*>(fake_original_loadfile);

  // Prevent the native test from touching a fixed firmware VA. The lazy hook
  // availability bookkeeping remains the production code's real path.
  std::call_once(g_process_install_once, [] {});
  g_process_tramp.target_addr = reinterpret_cast<void*>(uintptr_t{1});
  g_hook_degraded.store(false, std::memory_order_release);
  g_process_hook_enabled = true;
  g_loader = new NamLoader([](NamLoader::Request request) {
    request.payload();
  });

  test_disarmed_stock_path(valid_a.string());
  test_ordinary_compatibility_and_exceptions(valid_a.string());
  test_recognized_nam_failures(valid_a.string(), invalid.string());
  g_armed.store(true, std::memory_order_release);
  test_delayed_first_load(valid_a.string());
  test_synchronous_ready_and_reader_grace(valid_a.string(), valid_b.string());
  test_rapid_abc_supersession(valid_a.string(), valid_b.string(),
                              valid_c.string());
  test_ordinary_clear_cannot_be_resurrected(valid_a.string());
  test_queued_cancellation_releases_waiter_and_worker_recovers(
      valid_a.string(), valid_b.string(), valid_c.string());
  test_preparation_exception_leaves_worker_usable(invalid.string(),
                                                   valid_a.string());
  test_held_invalid_cleanup_preserves_newer(invalid.string(),
                                            valid_c.string());
  test_fresh_sequences_and_identical_captures(
      valid_a.string(), valid_b.string(), valid_c.string());
  test_stale_publication_cannot_erase_newer(valid_a.string(), valid_c.string());

  g_armed.store(false, std::memory_order_release);
  delete g_loader;
  g_loader = nullptr;
  fs::remove_all(work);
  std::cout << "NAM worker preparation, synchronous completion, cancellation, "
               "reader grace, and return ABI: PASS\n";
}
