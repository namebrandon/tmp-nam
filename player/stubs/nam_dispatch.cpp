// nam_dispatch.so — Tier-1 inline NAM dispatch via LD_PRELOAD.
// Design + status: docs/tmp-ir-slot-nam-tier1.md.
//
// Firmware target: 1.8.58 (owner's physical amp). All four IRProcessor VAs
// are confirmed and pinned to the engine SHA below; a version gate refuses to
// install on any other build. Recon: firmware/1.8.58/recon/irprocessor-derivation.md.
// Deploy over serial via a /run systemd drop-in — see docs/tmp-nam-status.md.

#define _GNU_SOURCE

#include <atomic>
#include <chrono>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <future>
#include <ios>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sched.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

// glibc may expose these legacy device-number macros from sys/types.h. NAM
// Core uses the same names for Version fields.
#ifdef major
#undef major
#endif
#ifdef minor
#undef minor
#endif

#include "NAM/dsp.h"
#include "NAM/get_dsp.h"
#include "nam_player.h"
#include "nam_model_config.h"
#include "nam_profile.h"
#include "nam_registry.h"
#include "nam_loader.h"
#include "nam_sha256.h"
#include <array>
#include <cmath>
#include <algorithm>

extern "C" {
#include "nam_dispatch_trampoline.h"
}

// ---------------------------------------------------------------------------
// Firmware 1.8.58 recon targets (firmware/1.8.58/recon/irprocessor-derivation.md)
//
// All four IRProcessor addresses are CONFIRMED for the owner's amp (1.8.58).
// tm-stomp-server is non-PIE, image base 0x400000; va_to_addr() rebases to the
// runtime load address. These MUST match the binary the version gate below
// pins to — never trust them against a different build.
// ---------------------------------------------------------------------------

// SHA-256 of the tm-stomp-server this stub was derived against. The
// constructor refuses to install trampolines unless /proc/self/exe matches.
#define TMP_1858_SHA \
  "e8f09d53662de56771f1cf76bb81835da86673a38bde5cdf9cb8ea82d94f2d38"

namespace {
// Confirmed 1.8.58 VAs (were 1.7.75: loadFile 0x55f2b0, process 0x6d6b20,
// ctor 0x95ef00 — every one moved ~525 KB; deploying the old values would
// trampoline arbitrary code, which is exactly what the SHA gate prevents).
constexpr uintptr_t kLoadFileVA = 0x9a47c0;  // IRProcessor::loadFile(this,path)
constexpr uintptr_t kProcessVA  = 0x9a5200;  // process(this,in,out,nframes)
constexpr uintptr_t kDtorVA     = 0x9a3ae0;  // IRProcessor D1; see versioned recon
}  // namespace

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

namespace {

std::mutex g_log_mutex;
thread_local int t_in_log = 0;

const char* log_path() {
  static const char* cached = nullptr;
  if (!cached) {
    const char* env = std::getenv("TMP_NAM_DISPATCH_LOG");
    cached = (env && env[0]) ? env : "/tmp/nam_dispatch.log";
  }
  return cached;
}

void logf(const char* fmt, ...) __attribute__((format(printf, 1, 2)));
void logf(const char* fmt, ...) {
  if (t_in_log) return;
  t_in_log = 1;
  std::lock_guard<std::mutex> lk(g_log_mutex);
  if (FILE* fh = std::fopen(log_path(), "a")) {
    va_list ap;
    va_start(ap, fmt);
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    std::fprintf(fh, "t=%llu.%03llu pid=%d tid=%ld ",
                 (unsigned long long)ts.tv_sec,
                 (unsigned long long)(ts.tv_nsec / 1000000),
                 getpid(), (long)syscall(SYS_gettid));
    std::vfprintf(fh, fmt, ap);
    va_end(ap);
    std::fputc('\n', fh);
    std::fclose(fh);
  }
  t_in_log = 0;
}

// ---------------------------------------------------------------------------
// Version gate — SHA-256 of /proc/self/exe (FIPS 180-4, no OpenSSL dep).
// Ported verbatim from nam_recon.c so both stubs share one implementation.
// The constructor installs nothing unless the running engine matches the
// build these VAs were derived against.
// ---------------------------------------------------------------------------

using namespace tmp_nam::detail;

// Hash /proc/self/exe; return true and fill hex[65] on success.
bool self_sha256(char hex[65]) {
  int fd = ::open("/proc/self/exe", O_RDONLY);
  if (fd < 0) {
    logf("version gate: open(/proc/self/exe) failed errno=%d", errno);
    return false;
  }
  sha256_ctx c;
  sha256_init(&c);
  uint8_t buf[65536];
  ssize_t r;
  while ((r = ::read(fd, buf, sizeof buf)) > 0)
    sha256_update(&c, buf, size_t(r));
  ::close(fd);
  if (r < 0) {
    logf("version gate: read(/proc/self/exe) failed errno=%d", errno);
    return false;
  }
  uint8_t out[32];
  sha256_final(&c, out);
  static const char* HX = "0123456789abcdef";
  for (int i = 0; i < 32; i++) {
    hex[i * 2] = HX[out[i] >> 4];
    hex[i * 2 + 1] = HX[out[i] & 0xf];
  }
  hex[64] = 0;
  return true;
}

// Returns true if the running engine matches the 1.8.58 recon key (or the gate
// is explicitly overridden). Logs the verdict either way.
bool version_gate_ok() {
  char hex[65];
  if (!self_sha256(hex)) {
    logf("version gate: cannot hash self — refusing to install.");
    return false;
  }
  bool match = std::strcmp(hex, TMP_1858_SHA) == 0;
  logf("version gate: self sha256=%s (%s)", hex,
       match ? "MATCH 1.8.58" : "MISMATCH");
  if (match) return true;
  if (const char* ov = std::getenv("TMP_NAM_DISPATCH_SKIP_VERSION_GATE")) {
    if (ov[0]) {
      logf("version gate: OVERRIDDEN via TMP_NAM_DISPATCH_SKIP_VERSION_GATE — "
           "VAs assume 1.8.58; proceed only if you know the binary matches.");
      return true;
    }
  }
  logf("version gate FAILED — this is not the 1.8.58 tm-stomp-server these "
       "VAs were derived against. NAM dispatch DISABLED. Set "
       "TMP_NAM_DISPATCH_SKIP_VERSION_GATE=1 to override (unsafe).");
  return false;
}

// ---------------------------------------------------------------------------
// Binary-base discovery (for log_offset, vtable validation)
// ---------------------------------------------------------------------------

uintptr_t g_binary_base = 0;
uintptr_t g_binary_end = 0;
char g_binary_name[128] = {0};

void discover_binary_base() {
  FILE* fh = std::fopen("/proc/self/maps", "r");
  if (!fh) return;
  char line[1024];
  while (std::fgets(line, sizeof line, fh)) {
    char* path = std::strchr(line, '/');
    if (!path) continue;
    if (!std::strstr(path, "tm-stomp-server")) continue;
    if (!std::strstr(line, " r-xp ")) continue;
    uintptr_t s = 0, e = 0;
    if (std::sscanf(line, "%lx-%lx", &s, &e) == 2) {
      g_binary_base = s;
      g_binary_end = e;
      char* nl = std::strchr(path, '\n');
      if (nl) *nl = '\0';
      char* base = std::strrchr(path, '/');
      const char* src = base ? base + 1 : path;
      std::strncpy(g_binary_name, src, sizeof g_binary_name - 1);
      break;
    }
  }
  std::fclose(fh);
}

// ---------------------------------------------------------------------------
// Stub WAV (unity IR, 4096 samples @ 44.1 kHz mono int16)
// ---------------------------------------------------------------------------

const char* g_stub_wav_path = "/data/.nam_dispatch_44100.wav";

bool ensure_stub_wav() {
  FILE* fh = std::fopen(g_stub_wav_path, "wb");
  if (!fh) {
    logf("ERROR: cannot create stub WAV at %s (errno=%d)", g_stub_wav_path,
         errno);
    return false;
  }
  const uint32_t sample_rate = 44100;
  const uint16_t channels = 1;
  const uint16_t bps = 16;
  // The pinned firmware copies its full 4096-frame IR buffer from the WAV
  // reader without bounding the copy to the file's sample count. A shorter
  // placeholder causes an out-of-bounds read during real loadFile calls.
  const uint32_t n_frames = 4096;
  const uint32_t data_size = n_frames * channels * (bps / 8);
  uint32_t file_size = 36 + data_size;
  uint32_t fmt_size = 16;
  uint16_t fmt = 1;
  uint16_t block_align = channels * (bps / 8);
  uint32_t byte_rate = sample_rate * block_align;

  std::fwrite("RIFF", 1, 4, fh);
  std::fwrite(&file_size, 4, 1, fh);
  std::fwrite("WAVE", 1, 4, fh);
  std::fwrite("fmt ", 1, 4, fh);
  std::fwrite(&fmt_size, 4, 1, fh);
  std::fwrite(&fmt, 2, 1, fh);
  std::fwrite(&channels, 2, 1, fh);
  std::fwrite(&sample_rate, 4, 1, fh);
  std::fwrite(&byte_rate, 4, 1, fh);
  std::fwrite(&block_align, 2, 1, fh);
  std::fwrite(&bps, 2, 1, fh);
  std::fwrite("data", 1, 4, fh);
  std::fwrite(&data_size, 4, 1, fh);
  std::vector<int16_t> silence(n_frames * channels, 0);
  // The stock loader initializes its wrapper with this neutral IR. A NAM load
  // does not report success until the fresh Player is also ready and bound.
  silence[0] = 32767;
  std::fwrite(silence.data(), 2, silence.size(), fh);
  bool written = std::ferror(fh) == 0;
  if (std::fclose(fh) != 0) written = false;
  if (!written) {
    logf("ERROR: failed writing stub WAV; NAM redirection stays disabled");
    return false;
  }
  logf("created stub WAV: %s (%u frames, unity IR)", g_stub_wav_path, n_frames);
  return true;
}

// ---------------------------------------------------------------------------
// Registries
// ---------------------------------------------------------------------------

constexpr int kEngineRate = 44100;
constexpr size_t kInstanceCapacity = 64;
std::atomic<size_t> g_live_entries{0};
struct NamEntry {
  NamEntry() { g_live_entries.fetch_add(1); }
  ~NamEntry() { g_live_entries.fetch_sub(1); }
  size_t slot = 0;
  uint64_t generation = 0;
  std::unique_ptr<tmp_nam::Player> player;
  std::string path, hash;
#ifdef TMP_NAM_DISPATCH_TEST
  std::thread::id test_control_thread;
  std::thread::id test_worker_thread;
#endif
  std::atomic_flag processing = ATOMIC_FLAG_INIT;
  std::atomic<uint64_t> blocks{0}, frames{0}, time_ns{0}, max_ns{0}, misses{0}, errors{0};
  std::atomic<uint64_t> src_underflows{0};
  tmp_nam::UtilizationHistogram utilization;
  // TMP_NAM_PROFILE=2 diagnostics. The audio thread is the only writer (plain
  // load/store, no RMW); the telemetry thread reads, logs per-window deltas
  // against diag_prev (its own snapshot), and resets the peaks.
  std::atomic<uint64_t> diag_blocks{0}, diag_zero_in_blocks{0}, diag_value_zero_in_blocks{0};
  std::atomic<uint64_t> diag_nonzero_in_samples{0}, diag_out_changes{0}, diag_skipped{0};
  float diag_last_out = 0.0f;  // audio thread only
  std::atomic<double> diag_in_sq{0.0}, diag_out_sq{0.0};
  std::atomic<float> diag_in_peak{0.0f}, diag_out_peak{0.0f};
  std::atomic<long> diag_tid{0};
  std::atomic<int> diag_cpu{-1};
  struct DiagCounters {
    uint64_t blocks = 0, zero_in_blocks = 0, value_zero_in_blocks = 0, nonzero_in_samples = 0;
    uint64_t out_changes = 0, skipped = 0, frames = 0;
    double in_sq = 0.0, out_sq = 0.0;
  } diag_prev;  // telemetry thread only

  DiagCounters load_diag() const {
    constexpr auto relaxed = std::memory_order_relaxed;
    return {diag_blocks.load(relaxed), diag_zero_in_blocks.load(relaxed), diag_value_zero_in_blocks.load(relaxed),
            diag_nonzero_in_samples.load(relaxed), diag_out_changes.load(relaxed), diag_skipped.load(relaxed),
            frames.load(relaxed), diag_in_sq.load(relaxed), diag_out_sq.load(relaxed)};
  }
};
tmp_nam::Registry<NamEntry, kInstanceCapacity> g_players;
using PrepareTask = std::packaged_task<std::shared_ptr<NamEntry>()>;
using NamLoader = tmp_nam::LatestLoader<PrepareTask, kInstanceCapacity>;
// This preload and its detached telemetry worker live for the engine process.
// Keep the existing loader alive too; static destruction must not race them.
NamLoader* g_loader = nullptr;
std::atomic<bool> g_armed{false};
std::atomic<bool> g_hook_degraded{false};
std::atomic<bool> g_profile{false};
// TMP_NAM_PROFILE=2: per-entry signal levels and thread/CPU placement. Only
// read inside the profiling branch, so the non-profiling callback is unchanged.
std::atomic<bool> g_diag{false};
thread_local int t_in_classify = 0;
thread_local bool t_in_loadfile = false;
thread_local bool t_loadfile_redirected_nam = false;

#ifdef TMP_NAM_DISPATCH_TEST
using PrepareGate = void (*)();
using BeforePublish = void (*)(
    void*, tmp_nam::Registry<NamEntry, kInstanceCapacity>::Ticket);
using AfterSubmit = void (*)(
    void*, tmp_nam::Registry<NamEntry, kInstanceCapacity>::Ticket);
PrepareGate g_prepare_gate = nullptr;
BeforePublish g_before_publish = nullptr;
AfterSubmit g_after_submit = nullptr;
#endif

bool is_nam_path(const char* p) {
  if (!p) return false;
  size_t n = std::strlen(p);
  return n >= 8 && std::strcmp(p + n - 8, ".nam.wav") == 0;
}

std::shared_ptr<NamEntry> prepare_instance(const std::string& path) {
  // Model I/O re-enters the preload's open interposers. Keep the real model
  // path out of the placeholder redirect while preparing on the loader worker.
  struct ClassifyScope {
    ClassifyScope() { ++t_in_classify; }
    ~ClassifyScope() { --t_in_classify; }
  } scope;
#ifdef TMP_NAM_DISPATCH_TEST
  if (g_prepare_gate) g_prepare_gate();
#endif
  tmp_nam::Options options;
  options.engine_rate = kEngineRate;
  const char* config = std::getenv("TMP_NAM_PLAYER_CONFIG");
  auto file = tmp_nam::read_model(
      path, options, config && config[0] ? config : "/data/nam/player.json");
  auto entry = std::make_shared<NamEntry>();
  // make_player performs allocation, reset and prewarm before this entry can
  // become callback-visible. Every load receives a fresh mutable Player.
  entry->player = tmp_nam::make_player(file);
  entry->path = path;
  entry->hash = file.hash;
  return entry;
}

bool ensure_process_trampoline();

const char* classify_and_redirect(const char* path) {
  if (!g_armed.load(std::memory_order_acquire) || t_in_classify) return path;
  // Only the explicit .nam.wav transport is intercepted. JSON parsing, hashes,
  // weights and metadata inspection happen in loadFile, not in file-open.
  // A normal .wav always goes to Fender, independent of previous NAM loads.
  if (!is_nam_path(path)) return path;
  if (!ensure_process_trampoline()) {
    g_armed.store(false, std::memory_order_release);
    logf("NAM dispatch DISABLED: process hook unavailable; original file is not redirected");
    return path;
  }
  if (t_in_loadfile) t_loadfile_redirected_nam = true;
  return g_stub_wav_path;
}

void telemetry_worker() {
  for (;;) {
    std::this_thread::sleep_for(std::chrono::seconds(5));
    const auto active = g_players.collect();
    logf("NAM registry active=%zu live=%zu pending=%zu loader_errors=%lu",
         active.size(), g_live_entries.load(),
         g_loader ? g_loader->outstanding() : 0,
         (unsigned long)(g_loader ? g_loader->errors() : 0));
    for (const auto& entry : active) {
      auto blocks = entry->blocks.load(std::memory_order_relaxed);
      if (!blocks) continue;
      if (!g_profile.load(std::memory_order_relaxed)) {
        logf("NAM stats sha256=%s blocks=%lu frames=%lu engine_rate=%d errors=%lu src_underflows=%lu slot=%zu generation=%lu profiling=off",
             entry->hash.c_str(), (unsigned long)blocks,
             (unsigned long)entry->frames.load(std::memory_order_relaxed), kEngineRate,
             (unsigned long)entry->errors.load(std::memory_order_relaxed),
             (unsigned long)entry->src_underflows.load(std::memory_order_relaxed),
             entry->slot, (unsigned long)entry->generation);
        continue;
      }
      const auto profile = entry->utilization.snapshot();
      logf("NAM stats sha256=%s blocks=%lu frames=%lu engine_rate=%d avg_us=%.3f "
           "max_us=%.3f p999_callback_pct_upper=%.1f p999_overflow=%d "
           "profile_samples=%lu profile_overflow=%lu "
           "deadline_misses=%lu errors=%lu src_underflows=%lu "
           "slot=%zu generation=%lu",
           entry->hash.c_str(), (unsigned long)blocks,
           (unsigned long)entry->frames.load(std::memory_order_relaxed), kEngineRate,
           entry->time_ns.load(std::memory_order_relaxed) / (1000.0 * blocks),
           entry->max_ns.load(std::memory_order_relaxed) / 1000.0,
           profile.p999_permille_upper / 10.0,
           profile.p999_overflow ? 1 : 0,
           (unsigned long)profile.samples,
           (unsigned long)profile.overflow,
           (unsigned long)entry->misses.load(std::memory_order_relaxed),
           (unsigned long)entry->errors.load(std::memory_order_relaxed),
           (unsigned long)entry->src_underflows.load(std::memory_order_relaxed),
           entry->slot, (unsigned long)entry->generation);
      if (!g_diag.load(std::memory_order_relaxed)) continue;
      // Per-window values: deltas against this thread's previous snapshot.
      const auto now = entry->load_diag();
      const auto& prev = entry->diag_prev;
      const uint64_t window_frames = now.frames - prev.frames;
      const double denom = window_frames ? static_cast<double>(window_frames) : 1.0;
      logf("NAM diag sha256=%.16s slot=%zu generation=%lu tid=%ld cpu=%d window_blocks=%lu "
           "in_zero_blocks=%lu in_value_zero_blocks=%lu in_nonzero_samples=%lu out_changes=%lu "
           "skipped_model_frames=%lu "
           "in_rms=%.3e in_peak=%.3e out_rms=%.3e out_peak=%.3e",
           entry->hash.c_str(), entry->slot, (unsigned long)entry->generation,
           entry->diag_tid.load(std::memory_order_relaxed),
           entry->diag_cpu.load(std::memory_order_relaxed),
           (unsigned long)(now.blocks - prev.blocks),
           (unsigned long)(now.zero_in_blocks - prev.zero_in_blocks),
           (unsigned long)(now.value_zero_in_blocks - prev.value_zero_in_blocks),
           (unsigned long)(now.nonzero_in_samples - prev.nonzero_in_samples),
           (unsigned long)(now.out_changes - prev.out_changes),
           (unsigned long)(now.skipped - prev.skipped),
           std::sqrt((now.in_sq - prev.in_sq) / denom),
           static_cast<double>(entry->diag_in_peak.exchange(0.0f, std::memory_order_relaxed)),
           std::sqrt((now.out_sq - prev.out_sq) / denom),
           static_cast<double>(entry->diag_out_peak.exchange(0.0f, std::memory_order_relaxed)));
      entry->diag_prev = now;
    }
  }
}

}  // namespace

// ---------------------------------------------------------------------------
// Hook 1: basic_filebuf::open interpose
// ---------------------------------------------------------------------------

// These symbols are meaningful only when this file is built as an LD_PRELOAD
// library.  The dispatch unit test links the implementation into its executable;
// older glibc cannot resolve that executable interposer with RTLD_NEXT, causing
// the test's own model reader to fail before it reaches the behavior under test.
#ifndef TMP_NAM_DISPATCH_TEST

using filebuf_open_t = void* (*)(void*, const char*, std::ios_base::openmode);
extern "C" void* _ZNSt13basic_filebufIcSt11char_traitsIcEE4openEPKcSt13_Ios_Openmode(
    void* self, const char* path, std::ios_base::openmode mode) {
  static const auto real_filebuf_open = reinterpret_cast<filebuf_open_t>(dlsym(
      RTLD_NEXT, "_ZNSt13basic_filebufIcSt11char_traitsIcEE4openEPKcSt13_Ios_Openmode"));
  if (!real_filebuf_open) { errno = ENOSYS; return nullptr; }
  return real_filebuf_open(self, classify_and_redirect(path), mode);
}

// ---------------------------------------------------------------------------
// C-API open hooks (fopen / fopen64 / open / open64)
// The WAV reader path may use any of these. .nam.wav files get redirected
// to the stub WAV during the single stock load call.
// ---------------------------------------------------------------------------

using fopen_t = FILE* (*)(const char*, const char*);
using open_t = int (*)(const char*, int, ...);

extern "C" FILE* fopen(const char* path, const char* mode) {
  static const auto real_fopen = reinterpret_cast<fopen_t>(dlsym(RTLD_NEXT, "fopen"));
  if (!real_fopen) { errno = ENOSYS; return nullptr; }
  return real_fopen(classify_and_redirect(path), mode);
}

extern "C" FILE* fopen64(const char* path, const char* mode) {
  static const auto real_fopen64 = [] {
    auto fn = reinterpret_cast<fopen_t>(dlsym(RTLD_NEXT, "fopen64"));
    return fn ? fn : reinterpret_cast<fopen_t>(dlsym(RTLD_NEXT, "fopen"));
  }();
  if (!real_fopen64) { errno = ENOSYS; return nullptr; }
  return real_fopen64(classify_and_redirect(path), mode);
}

extern "C" int open(const char* path, int flags, ...) {
  static const auto real_open = reinterpret_cast<open_t>(dlsym(RTLD_NEXT, "open"));
  if (!real_open) { errno = ENOSYS; return -1; }
  mode_t m = 0;
  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    m = (mode_t)va_arg(ap, int);
    va_end(ap);
  }
  return real_open(classify_and_redirect(path), flags, m);
}

extern "C" int open64(const char* path, int flags, ...) {
  static const auto real_open64 = [] {
    auto fn = reinterpret_cast<open_t>(dlsym(RTLD_NEXT, "open64"));
    return fn ? fn : reinterpret_cast<open_t>(dlsym(RTLD_NEXT, "open"));
  }();
  if (!real_open64) { errno = ENOSYS; return -1; }
  mode_t m = 0;
  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    m = (mode_t)va_arg(ap, int);
    va_end(ap);
  }
  return real_open64(classify_and_redirect(path), flags, m);
}
#endif

// ---------------------------------------------------------------------------
// Hooks 2 & 3: prologue trampolines
// ---------------------------------------------------------------------------

namespace {

uintptr_t va_to_addr(uintptr_t va) {
  // tm-stomp-server is non-PIE, image base 0x400000. binary_base will be
  // 0x400000 in practice; gating just in case.
  if (g_binary_base == 0) discover_binary_base();
  if (g_binary_base == 0) return va;
  return g_binary_base + (va - 0x400000);
}

struct tramp g_loadfile_tramp{};
struct tramp g_process_tramp{};
struct tramp g_dtor_tramp{};

// Lazy process-hook install. The process trampoline is NOT installed at boot —
// running the DSP real-time thread through it perturbs its spin-wait sync and
// made boot flaky (hung ~1 in 3). Instead it's installed the first time a NAM
// IR is actually loaded (loadFile of a .nam.wav), so stock/boot presets never
// touch it and boot is always reliable. g_process_hook_enabled mirrors the
// DISABLE_PROCESS gate; the install itself happens exactly once.
bool g_process_hook_enabled = false;
std::once_flag g_process_install_once;
bool ensure_process_trampoline();

// Both firmware callers consume the low return byte. Preserve it across our
// bookkeeping; the exact source type is not recovered from the binary.
using loadfile_result_t = std::uint8_t;
using loadfile_t = loadfile_result_t (*)(void*, void*);
// Confirmed 1.8.58 ABI: void process(this, float* in, float* out,
// size_t nframes) → x0/x1/x2/x3. We still declare 8 GP params so the tail-call
// through the saved prologue preserves x4..x7 untouched (harmless — the real
// function ignores them).
using process_t = void (*)(void*, void*, void*, void*, void*, void*, void*,
                           void*);
void irproc_dtor_handler(void* self) {
  const auto cleared = g_players.clear_details(self);
  if (cleared.cleared && g_loader)
    g_loader->cancel(cleared.index, cleared.generation);
  logf("IRProcessor destructor self=%p cleared_nam=%d slot=%zu generation=%lu pending_loads=%zu",
       self, cleared.cleared ? 1 : 0, cleared.index,
       (unsigned long)cleared.generation,
       g_loader ? g_loader->outstanding() : 0);
  reinterpret_cast<void (*)(void*)>(g_dtor_tramp.exec_buffer)(self);
}

using NamTicket = tmp_nam::Registry<NamEntry, kInstanceCapacity>::Ticket;

// A failed NAM request: drop the binding its own generation owns, so Fender's
// path (the unity placeholder) runs instead. A newer load or an ordinary clear
// on the same processor is never disturbed.
bool unbind_failed_nam(NamTicket ticket) {
  if (g_loader) g_loader->cancel(ticket.index, ticket.generation);
  return g_players.clear_if_current(ticket);
}

loadfile_result_t loadfile_handler(void* self, void* str_ref) {
  // The pinned engine passes its C++11 std::string object at this boundary.
  // Inspect the request itself so a failed redirect cannot be misclassified as
  // an ordinary successful load. The disarmed path remains a pure stock call.
  const bool armed_at_entry = g_armed.load(std::memory_order_acquire);
  const auto* requested =
      armed_at_entry && str_ref
          ? static_cast<const std::string*>(str_ref)
          : nullptr;
  const bool requested_nam = requested && is_nam_path(requested->c_str());
  // A NAM request reserves its generation before the stock call, so whatever it
  // later clears on failure is its own binding, never a newer one.
  NamTicket ticket{0, 0, false};
  bool ticket_started = false;
  const char* reservation_failure = nullptr;
  if (requested_nam) {
    try {
      ticket = g_players.begin(self);
      ticket_started = true;
    } catch (const std::exception& error) {
      reservation_failure = error.what();
    }
  }
  t_loadfile_redirected_nam = false;
  t_in_loadfile = true;
  loadfile_result_t original_result;
  try {
    original_result = reinterpret_cast<loadfile_t>(g_loadfile_tramp.exec_buffer)(self, str_ref);
  } catch (...) {
    t_in_loadfile = false;
    t_loadfile_redirected_nam = false;
    // Ordinary stock loads continue to remove a NAM binding. A recognized NAM
    // attempt keeps the previous owner alive while preserving the stock
    // loader's original exception type.
    if (armed_at_entry && !requested_nam) {
      const auto cleared = g_players.clear_details(self);
      if (cleared.cleared && g_loader)
        g_loader->cancel(cleared.index, cleared.generation);
    }
    if (ticket_started) {
      if (g_loader) g_loader->cancel(ticket.index, ticket.generation);
      (void)g_players.cancel(ticket);
    }
    throw;  // preserve Fender's original exception behavior
  }
  t_in_loadfile = false;
  const bool redirected_nam = t_loadfile_redirected_nam;
  t_loadfile_redirected_nam = false;
  if (!armed_at_entry) return original_result;
  if (!requested_nam) {
    const auto cleared = g_players.clear_details(self);
    if (cleared.cleared && g_loader)
      g_loader->cancel(cleared.index, cleared.generation);
    logf("ordinary IR restored self=%p cleared_nam=%d slot=%zu generation=%lu pending_loads=%zu",
         self, cleared.cleared ? 1 : 0, cleared.index,
         (unsigned long)cleared.generation,
         g_loader ? g_loader->outstanding() : 0);
    return original_result;
  }

  // A recognized NAM request binds a Player only when the stock wrapper used the
  // placeholder, the process hook is available, and a fresh Player is published
  // before this returns, so the inactive bank can never go live with a late swap.
  // Any other outcome unbinds this processor (Fender's path then runs the unity
  // placeholder: an audible bypass) and returns the stock result. It never throws:
  // the firmware builds the IR unit around this call, and an exception there
  // leaves the unit half-built, so its pooled IRProcessor is never released.
  const std::string requested_path = *requested;
  const char* early_failure =
      reservation_failure                                      ? reservation_failure
      : !redirected_nam || !g_armed.load(std::memory_order_acquire) ||
              !ensure_process_trampoline()                     ? "NAM process dispatch is unavailable"
      : original_result == 0                                   ? "stock NAM placeholder load failed"
                                                               : nullptr;
  if (early_failure) {
    const bool unbound = ticket_started && unbind_failed_nam(ticket);
    logf("NAM load failed path=%s self=%p unbound=%d: %s", requested_path.c_str(),
         self, unbound ? 1 : 0, early_failure);
    return original_result;
  }

  std::string failure;
  try {
    // The generation covers the entire preparation interval. If a newer NAM
    // load or an ordinary clear wins while this work is in progress, publish
    // rejects this exact ticket and the failure cleanup cannot disturb the
    // newer state.
#ifdef TMP_NAM_DISPATCH_TEST
    const auto control_thread_id = std::this_thread::get_id();
    PrepareTask task([path = requested_path, control_thread_id]() {
      auto entry = prepare_instance(path);
      entry->test_control_thread = control_thread_id;
      entry->test_worker_thread = std::this_thread::get_id();
      return entry;
    });
#else
    PrepareTask task(
        [path = requested_path]() { return prepare_instance(path); });
#endif
    auto ready = task.get_future();
    if (!g_loader ||
        !g_loader->submit(ticket.index, ticket.generation, std::move(task)))
      throw std::runtime_error("NAM loader request rejected");
    logf("NAM load queued self=%p path=%s slot=%zu generation=%lu "
         "previous_ready=%d",
         self, requested_path.c_str(), ticket.index,
         (unsigned long)ticket.generation, ticket.had_ready ? 1 : 0);
#ifdef TMP_NAM_DISPATCH_TEST
    if (g_after_submit) g_after_submit(self, ticket);
#endif
    auto entry = ready.get();
    entry->slot = ticket.index;
    entry->generation = ticket.generation;
#ifdef TMP_NAM_DISPATCH_TEST
    if (g_before_publish) g_before_publish(self, ticket);
#endif
    if (!g_players.publish(ticket, entry))
      throw std::runtime_error("stale NAM publication rejected");
    logf("NAM ready before load return path=%s sha256=%s engine_rate=%d "
         "model_rate=%d latency_frames=%zu slot=%zu generation=%lu impl=%s",
         entry->path.c_str(), entry->hash.c_str(), kEngineRate,
         entry->player->model_rate(),
         static_cast<size_t>(entry->player->latency_frames()), entry->slot,
         (unsigned long)entry->generation, entry->player->implementation());
    return original_result;
  } catch (const std::exception& error) {
    failure = error.what();
  } catch (...) {
    failure = "unknown error";
  }
  const bool unbound = unbind_failed_nam(ticket);
  logf("NAM load failed path=%s slot=%zu generation=%lu unbound=%d: %s",
       requested_path.c_str(), ticket.index, (unsigned long)ticket.generation,
       unbound ? 1 : 0, failure.c_str());
  return original_result;
}

inline void chain_original_process(void* self, void* a1, void* a2, void* a3,
                                   void* a4, void* a5, void* a6, void* a7) {
  reinterpret_cast<process_t>(g_process_tramp.exec_buffer)(self, a1, a2, a3, a4, a5, a6, a7);
}

// The stock process on this processor's own input into a per-thread scratch
// output (the firmware calls with 32-frame blocks; longer calls are chunked).
void run_stock_shadow(void* self, float* in, size_t frames) {
  constexpr size_t kShadowFrames = 1024;
  thread_local float scratch[kShadowFrames];
  for (size_t offset = 0; offset < frames; offset += kShadowFrames) {
    const size_t n = std::min(kShadowFrames, frames - offset);
    chain_original_process(self, in + offset, scratch,
                           reinterpret_cast<void*>(static_cast<uintptr_t>(n)),
                           nullptr, nullptr, nullptr, nullptr);
  }
}

// TMP_NAM_PROFILE=2 helpers: sum of squares, peak, "every sample is +0.0" and
// the count of non-zero samples.
struct DiagScan {
  double sq = 0.0;
  float peak = 0.0f;
  bool zero = true;
  uint32_t nonzero = 0;
};

DiagScan diag_scan(const float* data, size_t frames) {
  DiagScan r;
  for (size_t i = 0; i < frames; ++i) {
    const float x = data[i];
    uint32_t bits;
    std::memcpy(&bits, &x, sizeof bits);
    r.zero = r.zero && bits == 0;
    r.nonzero += x != 0.0f;
    r.sq += static_cast<double>(x) * x;
    r.peak = std::max(r.peak, std::fabs(x));
  }
  return r;
}

void diag_record(NamEntry* entry, const float* out, size_t frames, const DiagScan& in) {
  const DiagScan o = diag_scan(out, frames);
  // Count output samples whose bits differ from the previous sample: 0 means the
  // output was one exact constant for the whole window.
  uint64_t changes = 0;
  float last = entry->diag_last_out;
  for (size_t i = 0; i < frames; ++i) {
    changes += std::memcmp(&last, &out[i], sizeof last) != 0;
    last = out[i];
  }
  entry->diag_last_out = last;
  constexpr auto relaxed = std::memory_order_relaxed;
  const uint64_t blocks = entry->diag_blocks.load(relaxed) + 1;
  entry->diag_blocks.store(blocks, relaxed);
  if (in.zero) entry->diag_zero_in_blocks.store(entry->diag_zero_in_blocks.load(relaxed) + 1, relaxed);
  if (!in.nonzero)
    entry->diag_value_zero_in_blocks.store(entry->diag_value_zero_in_blocks.load(relaxed) + 1, relaxed);
  entry->diag_nonzero_in_samples.store(entry->diag_nonzero_in_samples.load(relaxed) + in.nonzero, relaxed);
  entry->diag_out_changes.store(entry->diag_out_changes.load(relaxed) + changes, relaxed);
  entry->diag_skipped.store(entry->player->skipped_model_frames(), relaxed);
  entry->diag_in_sq.store(entry->diag_in_sq.load(relaxed) + in.sq, relaxed);
  entry->diag_out_sq.store(entry->diag_out_sq.load(relaxed) + o.sq, relaxed);
  // A peak lost to a concurrent telemetry reset only shortens that window.
  if (in.peak > entry->diag_in_peak.load(relaxed)) entry->diag_in_peak.store(in.peak, relaxed);
  if (o.peak > entry->diag_out_peak.load(relaxed)) entry->diag_out_peak.store(o.peak, relaxed);
  if ((blocks & 1023) == 1) {
    static thread_local const long tid = static_cast<long>(syscall(SYS_gettid));
    entry->diag_tid.store(tid, relaxed);
#ifdef __linux__
    entry->diag_cpu.store(sched_getcpu(), relaxed);
#endif
  }
}

void process_handler(void* self, void* a1, void* a2, void* a3,
                     void* a4, void* a5, void* a6, void* a7) {
  const bool profile = g_profile.load(std::memory_order_relaxed);
  const auto start = profile ? std::chrono::steady_clock::now() :
                              std::chrono::steady_clock::time_point{};
  auto state = g_players.read(self);
  if (!state.bound()) {
    chain_original_process(self, a1, a2, a3, a4, a5, a6, a7);
    return;
  }
  auto* in = static_cast<float*>(a1);
  auto* out = static_cast<float*>(a2);
  const size_t frames = reinterpret_cast<uintptr_t>(a3);
  if (!in || !out || !frames) return;
  // Fender's IR processing keeps running on the unity placeholder underneath the
  // NAM, output discarded. Its output ring is never cleared by loadIR, so a
  // processor frozen while a NAM played would replay the previous IR's pending
  // tail when an ordinary IR takes over. Runs first: the NAM may process in place.
  run_stock_shadow(self, in, frames);
  NamEntry* entry = state.get();
  // Explicit bypass during initial loading/failure. Never process a silent or
  // stale stock IR merely because a registry lock happens to be contended.
  if (!entry) {
    if (in != out) std::memmove(out, in, frames * sizeof(float));
    return;
  }
  if (entry->processing.test_and_set(std::memory_order_acquire)) {
    entry->errors.fetch_add(1, std::memory_order_relaxed);
    if (in != out) std::memmove(out, in, frames * sizeof(float));
    return;
  }
  // Diagnostics read the input before process(), which may overwrite it in place;
  // the scan's own time is taken out of the profiled call.
  const bool diag = profile && g_diag.load(std::memory_order_relaxed);
  DiagScan diag_in;
  int64_t diag_scan_ns = 0;
  if (diag) {
    const auto scan_start = std::chrono::steady_clock::now();
    diag_in = diag_scan(in, frames);
    diag_scan_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now() - scan_start).count();
  }
  try {
    entry->player->process(in, out, frames);
  } catch (...) {
    entry->errors.fetch_add(1, std::memory_order_relaxed);
    if (in != out) std::memmove(out, in, frames * sizeof(float));
  }
  entry->src_underflows.store(entry->player->late_underflow_frames(), std::memory_order_relaxed);
  const auto end = profile ? std::chrono::steady_clock::now() : std::chrono::steady_clock::time_point{};
  // Diagnostics are recorded while this callback still owns the entry: their
  // single-writer counters and the Player read must not race another callback.
  if (diag) diag_record(entry, out, frames, diag_in);
  entry->processing.clear(std::memory_order_release);
  entry->frames.fetch_add(frames, std::memory_order_relaxed);
  entry->blocks.fetch_add(1, std::memory_order_relaxed);
  if (!profile) return;
  const auto elapsed = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count() - diag_scan_ns;
  const auto ns = static_cast<uint64_t>(std::max<int64_t>(elapsed, 0));
  entry->time_ns.fetch_add(ns, std::memory_order_relaxed);
  uint64_t maximum = entry->max_ns.load(std::memory_order_relaxed);
  while (ns > maximum && !entry->max_ns.compare_exchange_weak(maximum, ns, std::memory_order_relaxed)) {}
  if (ns > static_cast<uint64_t>(frames) * 1000000000ULL / kEngineRate)
    entry->misses.fetch_add(1, std::memory_order_relaxed);
  entry->utilization.observe(ns, frames, kEngineRate);
}

// S4 mitigation: convert silent VM-kernel mprotect-on-text failures into a
// loud, early error. The VM's custom 6.12.x kernel may enforce W^X on
// private mappings; the production trampoline install would then fail with
// rc=-4 from far inside startup. The self-test reproduces the exact mprotect
// chain on a self-allocated page so any W^X gating surfaces before we touch
// real text. Skip via TMP_NAM_DISPATCH_SKIP_SELFTEST=1.
bool selftest_trampoline() {
  const size_t page_size = (size_t)sysconf(_SC_PAGESIZE);

  // Allocate two anonymous pages — one for the trampoline target (mimics
  // tm-stomp-server's .text), one for the handler we redirect to.
  void* target_page = mmap(nullptr, page_size, PROT_READ | PROT_WRITE,
                           MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  if (target_page == MAP_FAILED) {
    logf("trampoline selftest: mmap(target) failed errno=%d", errno);
    return false;
  }
  void* handler_page = mmap(nullptr, page_size, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  if (handler_page == MAP_FAILED) {
    logf("trampoline selftest: mmap(handler) failed errno=%d", errno);
    munmap(target_page, page_size);
    return false;
  }

  // Write 6 NOPs + RET to the target. tramp_install will copy the first
  // 5 instructions (all NOPs — non-PC-relative, accepted by the installer)
  // and overwrite them with a long-branch via x16. The 6th NOP + RET
  // remain for chain-back. Encoding: nop=0xd503201f, ret=0xd65f03c0.
  const uint32_t target_code[] = {0xd503201fu, 0xd503201fu, 0xd503201fu,
                                  0xd503201fu, 0xd503201fu, 0xd503201fu,
                                  0xd65f03c0u};
  std::memcpy(target_page, target_code, sizeof(target_code));

  // Tiny handler: just `ret`. We do not invoke the trampolined function in
  // the self-test — `tramp_install` returning rc=0 is the proof we wanted
  // (it means mmap + the in-place mprotect promote/restore on `target_page`
  // both succeeded, which is the exact chain that runs against real text).
  const uint32_t handler_code[] = {0xd65f03c0u};
  std::memcpy(handler_page, handler_code, sizeof(handler_code));

  // Promote both pages to R+X. If the kernel enforces strict W^X on
  // anonymous mappings this fails first.
  if (mprotect(target_page, page_size, PROT_READ | PROT_EXEC) != 0) {
    logf("trampoline selftest: mprotect(target R+X) failed errno=%d", errno);
    munmap(target_page, page_size);
    munmap(handler_page, page_size);
    return false;
  }
  if (mprotect(handler_page, page_size, PROT_READ | PROT_EXEC) != 0) {
    logf("trampoline selftest: mprotect(handler R+X) failed errno=%d", errno);
    munmap(target_page, page_size);
    munmap(handler_page, page_size);
    return false;
  }

  struct tramp test_tramp{};
  int rc = tramp_install_atomic(&test_tramp, target_page, handler_page);
  if (rc != 0) {
    logf("trampoline selftest: tramp_install rc=%d errno=%d — "
         "kernel may forbid W+X transitions on this fd or W^X is strict",
         rc, errno);
    munmap(target_page, page_size);
    munmap(handler_page, page_size);
    return false;
  }

  logf("trampoline selftest: PASS");
  // Pages + test_tramp.exec_buffer are not unmapped — they're harmless and
  // unmapping after a successful install could trip the kernel's check on
  // any future invocation. One-shot cost.
  return true;
}

// Install the process trampoline exactly once, on the first NAM IR load. Runs
// on the control (loadFile) thread — the deferred half of the lazy install.
bool ensure_process_trampoline() {
  if (!g_process_hook_enabled) return false;
  std::call_once(g_process_install_once, []() {
    void* addr = reinterpret_cast<void*>(va_to_addr(kProcessVA));
    int rc = tramp_install_atomic(&g_process_tramp, addr,
                           reinterpret_cast<void*>(process_handler));
    if (rc == TRAMP_INSTALL_ATOMIC_DEGRADED) {
      g_hook_degraded.store(true, std::memory_order_release);
      logf("ERROR: process hook live but target RX restoration failed; NAM redirection disabled");
    }
    logf("LAZY tramp_install IRProcessor::process (VA 0x%lx → addr %p) rc=%d",
         (unsigned long)kProcessVA, addr, rc);
  });
  return g_process_tramp.target_addr != nullptr && !g_hook_degraded.load(std::memory_order_acquire);
}

void install_trampolines() {
  // Layer 1 — version gate. These VAs are only valid for the exact 1.8.58
  // tm-stomp-server they were derived against; on any other build they would
  // trampoline arbitrary code. Refuse unless /proc/self/exe SHA-256 matches
  // (override with TMP_NAM_DISPATCH_SKIP_VERSION_GATE=1 — unsafe).
  if (!version_gate_ok()) {
    logf("NAM dispatch DISABLED — version gate declined. No .text touched.");
    return;
  }

  // Layer 2 — S4 self-test: bail loudly if the kernel's mprotect rules forbid
  // the trampoline mechanism. Set TMP_NAM_DISPATCH_SKIP_SELFTEST=1 to bypass
  // (useful only if the self-test is itself buggy on a given kernel — the
  // real-text install will still attempt mprotect).
  if (std::getenv("TMP_NAM_DISPATCH_SKIP_SELFTEST") == nullptr) {
    if (!selftest_trampoline()) {
      logf("trampoline selftest FAILED — declining to install trampolines "
           "on tm-stomp-server text. NAM dispatch is DISABLED for this run. "
           "Set TMP_NAM_DISPATCH_SKIP_SELFTEST=1 to override.");
      return;
    }
  }

  // D1 is reached directly and through the deleting destructor. Patch only
  // its entry instruction; no prologue skipping or concurrent multiword writes.
  int dtor_rc = tramp_install_atomic(&g_dtor_tramp,
      reinterpret_cast<void*>(va_to_addr(kDtorVA)), reinterpret_cast<void*>(irproc_dtor_handler));
  logf("tramp_install IRProcessor::D1 rc=%d", dtor_rc);
  if (dtor_rc != 0) return;

  // Confirmed 1.8.58 IRProcessor entries (see recon doc / constants at top).
  uintptr_t loadfile_va = kLoadFileVA;
  void* loadfile_addr = reinterpret_cast<void*>(va_to_addr(loadfile_va));

  if (std::getenv("TMP_NAM_DISPATCH_DISABLE_LOADFILE") == nullptr) {
    int rc = tramp_install_atomic(&g_loadfile_tramp, loadfile_addr,
                           reinterpret_cast<void*>(loadfile_handler));
    if (rc == TRAMP_INSTALL_ATOMIC_DEGRADED) {
      g_hook_degraded.store(true, std::memory_order_release);
      logf("ERROR: loadFile hook live but target RX restoration failed; NAM redirection disabled");
    }
    logf("tramp_install IRProcessor::loadFile (VA 0x%lx → addr %p) rc=%d",
         (unsigned long)loadfile_va, loadfile_addr, rc);
  }

  // process is installed LAZILY (ensure_process_trampoline), on the first NAM
  // IR load — NOT here — so the DSP real-time thread never runs through our
  // hook for stock/boot presets, keeping boot reliable.
  if (std::getenv("TMP_NAM_DISPATCH_DISABLE_PROCESS") == nullptr) {
    g_process_hook_enabled = true;
    logf("IRProcessor::process hook ENABLED (lazy: installs on first NAM IR)");
  } else {
    logf("IRProcessor::process hook DISABLED via env");
  }


}

}  // namespace

// Deferred arm: runs on a detached thread spawned by the constructor. It waits
// out the engine's timing-sensitive boot window, THEN does everything that used
// to run in the constructor (SHA-256 version gate, selftest, .text trampoline
// install). Doing that heavy work in the constructor — during boot — perturbed
// the engine's DSP-sync startup handshake and intermittently hung boot. Deferring
// it makes the stub 100% passive during boot, so the engine boots stock-identical.
void arm_worker() {
  const char* profile = std::getenv("TMP_NAM_PROFILE");
  // "1" profiles the NAM call; "2" also logs per-entry levels and thread/CPU.
  const bool diag = profile && std::strcmp(profile, "2") == 0;
  g_profile.store(profile && (std::strcmp(profile, "1") == 0 || diag), std::memory_order_relaxed);
  g_diag.store(diag, std::memory_order_relaxed);
  int delay_s = 15;  // seconds to let boot settle before touching anything
  if (const char* d = std::getenv("TMP_NAM_DISPATCH_ARM_DELAY"))
    if (d[0]) {
      int v = std::atoi(d);
      if (v >= 0 && v <= 120) delay_s = v;
    }
  logf("arm: waiting %ds for engine boot to settle before installing hooks",
       delay_s);
  std::this_thread::sleep_for(std::chrono::seconds(delay_s));

  if (!ensure_stub_wav()) return;

  // NAM Core's upstream approximation removes the dominant libm tanh cost on
  // Cortex-A57. It affects Tanh models (A1); the supplied A2 uses LeakyReLU.
  nam::activations::Activation::enable_fast_tanh();
  logf("NAM upstream fast tanh enabled");

  // mlockall pins our pages so the RT process path never page-faults. Disable
  // with TMP_NAM_DISPATCH_NO_MLOCK=1 (the shipped drop-in sets it — MCL_FUTURE
  // was itself a boot-hang source).
  if (std::getenv("TMP_NAM_DISPATCH_NO_MLOCK") == nullptr) {
    if (mlockall(MCL_CURRENT | MCL_FUTURE) == 0)
      logf("mlockall OK — pages pinned (no RT page faults on the process path)");
    else
      logf("mlockall FAILED errno=%d — RT page-fault risk remains", errno);
  }

  install_trampolines();  // version gate + selftest + loadFile install

  if (std::getenv("TMP_NAM_DISPATCH_PRELOAD_DIR") ||
      std::getenv("TMP_NAM_DISPATCH_DRIVE_PROCESS"))
    logf("Legacy preload/drive disabled; use the standalone nam_parity_test with per-instance Player");

  // Arm LAST: only now do the open hooks start classifying/redirecting .nam.wav
  // and only now are the trampolines live.
  if (!g_loadfile_tramp.target_addr || !g_dtor_tramp.target_addr || !g_process_hook_enabled ||
      g_hook_degraded.load(std::memory_order_acquire)) return;
  try {
    g_loader = new NamLoader([](NamLoader::Request request) {
      request.payload();
    });
  } catch (const std::exception& error) {
    logf("NAM dispatch DISABLED: cannot start loader: %s", error.what());
    return;
  }
  g_armed.store(true, std::memory_order_release);
  std::thread(telemetry_worker).detach();
  logf("arm: NAM dispatch ARMED (synchronous completion via existing "
       "non-RT loader worker)");
}

#ifndef TMP_NAM_DISPATCH_TEST
__attribute__((constructor)) static void nam_dispatch_init() {
  // Keep the constructor FAST and side-effect-free on the engine: discover the
  // image base (reads /proc/self/maps) and log, then hand everything heavy to a
  // detached thread. Nothing here may touch the engine's .text or block on I/O —
  // it runs mid-boot and any delay/patch here can hang the DSP-sync handshake.
  discover_binary_base();
  logf("=== nam_dispatch init pid=%d binary=%s base=0x%lx end=0x%lx ===",
       getpid(), g_binary_name, (unsigned long)g_binary_base,
       (unsigned long)g_binary_end);
  if (g_binary_base == 0) {
    // Pre-exec phase (e.g., entrypoint shell) or an LD_PRELOAD-inheriting helper
    // child, not the server — install nothing, spawn no arm thread.
    return;
  }
  std::thread(arm_worker).detach();
}
#endif
