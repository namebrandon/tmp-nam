#define _GNU_SOURCE
#include "nam_dispatch_trampoline.h"

#include <errno.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define TRAMP_PAGE_SIZE 0x1000UL
#define TRAMP_BRANCH_MAX_DISTANCE 0x08000000LL
#define TRAMP_NEAR_ATTEMPTS 16

#if defined(__linux__) && !defined(MAP_FIXED_NOREPLACE)
#define MAP_FIXED_NOREPLACE 0x100000
#endif

static int is_pc_relative(uint32_t instr) {
  if ((instr & 0x1F000000) == 0x10000000) return 1; /* ADR, ADRP */
  if ((instr & 0x7C000000) == 0x14000000) return 1; /* B, BL */
  if ((instr & 0xFF000010) == 0x54000000) return 1; /* B.cond */
  if ((instr & 0x7E000000) == 0x34000000) return 1; /* CBZ, CBNZ */
  if ((instr & 0x7E000000) == 0x36000000) return 1; /* TBZ, TBNZ */
  if ((instr & 0x3B000000) == 0x18000000) return 1; /* LDR literal (incl. SIMD) */
  return 0;
}

static void emit_long_branch(uint32_t out[5], uintptr_t dest) {
  /* movz x16, #imm16, lsl #0  → 0xD2800010 | (imm16 << 5) */
  /* movk x16, #imm16, lsl #16 → 0xF2A00010 | (imm16 << 5) */
  /* movk x16, #imm16, lsl #32 → 0xF2C00010 | (imm16 << 5) */
  /* movk x16, #imm16, lsl #48 → 0xF2E00010 | (imm16 << 5) */
  /* br   x16                  → 0xD61F0200 */
  out[0] = 0xD2800010u | (((uint32_t)((dest >>  0) & 0xFFFF)) << 5);
  out[1] = 0xF2A00010u | (((uint32_t)((dest >> 16) & 0xFFFF)) << 5);
  out[2] = 0xF2C00010u | (((uint32_t)((dest >> 32) & 0xFFFF)) << 5);
  out[3] = 0xF2E00010u | (((uint32_t)((dest >> 48) & 0xFFFF)) << 5);
  out[4] = 0xD61F0200u;
}

/* Encode the single instruction used to publish the NAM hook.  A64 B has a
 * signed, word-aligned, +/-128 MiB range. */
static int encode_b_imm26(uint32_t *out, uintptr_t pc, uintptr_t dest) {
  int64_t delta;
  if (!out || (pc & 3u) != 0 || (dest & 3u) != 0) return -1;
  if (dest >= pc) {
    uint64_t distance = (uint64_t)dest - (uint64_t)pc;
    if (distance > (uint64_t)(TRAMP_BRANCH_MAX_DISTANCE - 4)) return -1;
    delta = (int64_t)distance;
  } else {
    uint64_t distance = (uint64_t)pc - (uint64_t)dest;
    if (distance > (uint64_t)TRAMP_BRANCH_MAX_DISTANCE) return -1;
    delta = -(int64_t)distance;
  }
  if ((delta & 3) != 0) return -1;
  *out = 0x14000000u | ((uint32_t)(delta >> 2) & 0x03ffffffu);
  return 0;
}

/* Expand ADR/ADRP to four MOV-immediate instructions without touching flags or
 * any register other than Rd. The copied instruction's ORIGINAL PC is used. */
static int relocate_prologue_count(uint32_t out[20], const uint8_t saved[20],
                                   uintptr_t original, int allow_adr,
                                   int instruction_count) {
  if (!out || !saved || instruction_count < 0 || instruction_count > 5)
    return -1;
  int words = 0;
  for (int i = 0; i < instruction_count; ++i) {
    uint32_t instr;
    memcpy(&instr, saved + i * 4, 4);
    if (allow_adr && (instr & 0x1f000000u) == 0x10000000u) {
      uint32_t imm21 = ((instr >> 5) & 0x7ffffu) << 2 | ((instr >> 29) & 3u);
      int64_t offset = (int64_t)imm21 - ((imm21 & 0x100000u) ? 0x200000 : 0);
      uintptr_t pc = original + (uintptr_t)i * 4;
      if (instr & 0x80000000u) {
        pc &= ~(uintptr_t)0xfff;
        offset *= 4096;
      }
      uint64_t value = (uint64_t)pc + (uint64_t)offset;
      uint32_t rd = instr & 31u;
      out[words++] = 0xd2800000u | rd | ((value & 0xffffu) << 5);
      out[words++] = 0xf2a00000u | rd | (((value >> 16) & 0xffffu) << 5);
      out[words++] = 0xf2c00000u | rd | (((value >> 32) & 0xffffu) << 5);
      out[words++] = 0xf2e00000u | rd | (((value >> 48) & 0xffffu) << 5);
    } else {
      if (is_pc_relative(instr)) return -1;
      out[words++] = instr;
    }
  }
  return words;
}

static int relocate_prologue(uint32_t out[20], const uint8_t saved[20],
                             uintptr_t original, int allow_adr) {
  return relocate_prologue_count(out, saved, original, allow_adr, 5);
}

/* The long branch-back uses x16 as its scratch register.  Reject a first
 * instruction whose destination is x16, since the original code may consume
 * that value immediately after target+4.  This intentionally errs on the
 * conservative side for encodings whose low register field is not a
 * destination (for example a store). */
static int first_instruction_writes_x16(uint32_t instr) {
  if ((instr & 31u) == 16u) return 1;
  /* Load-pair destinations include Rt2 in bits 14:10.  Reject both load and
   * store forms here; the latter is conservative and keeps this validator
   * independent of every A64 load/store encoding variant. */
  if ((instr & 0x3a000000u) == 0x28000000u &&
      (((instr >> 10) & 31u) == 16u)) return 1;
  return 0;
}

static int atomic_first_supported(uint32_t instr) {
  const int is_adr = (instr & 0x1f000000u) == 0x10000000u;
  if (is_pc_relative(instr) && !is_adr) return 0;
  /* A continuation beginning with an indirect branch or exception return
   * cannot be safely resumed through target+4. */
  if ((instr & 0xfffffc1fu) == 0xd61f0000u ||  /* br   xn */
      (instr & 0xfffffc1fu) == 0xd63f0000u ||  /* blr  xn */
      (instr & 0xfffffc1fu) == 0xd65f0000u ||  /* ret  xn */
      (instr & 0xffc00000u) == 0xd4000000u ||  /* svc/brk family */
      (instr & 0xfffffc00u) == 0xd69f0000u ||  /* eret family */
      (instr & 0xfffffc00u) == 0xd6bf0000u)   /* drps */
    return 0;
  if (first_instruction_writes_x16(instr)) return 0;
  return 1;
}

static void *mmap_near_target(uintptr_t target, size_t length) {
#if defined(__linux__)
  /* A64's one-instruction branch needs a proxy within +/-128 MiB.  A normal
   * mmap hint may be ignored when the firmware heap covers the hinted page,
   * so search for an actually free page without replacing an existing map.
   * Try a coarse pass first; the page pass handles narrow holes. */
  const uintptr_t target_page = target & ~(uintptr_t)(TRAMP_PAGE_SIZE - 1);
  static const uintptr_t steps[] = {0x10000u, TRAMP_PAGE_SIZE};
  for (size_t pass = 0; pass < sizeof steps / sizeof steps[0]; ++pass) {
    const uintptr_t step = steps[pass];
    for (uintptr_t distance = step;
         distance < (uintptr_t)TRAMP_BRANCH_MAX_DISTANCE;
         distance += step) {
      uintptr_t candidates[2];
      size_t count = 0;
      if (target_page <= UINTPTR_MAX - distance)
        candidates[count++] = target_page + distance;
      if (target_page >= distance + TRAMP_PAGE_SIZE)
        candidates[count++] = target_page - distance;
      for (size_t index = 0; index < count; ++index) {
        uint32_t ignored;
        if (encode_b_imm26(&ignored, target, candidates[index] + 36) != 0)
          continue;
        void *mapped = mmap((void *)candidates[index], length,
                            PROT_READ | PROT_WRITE,
                            MAP_ANONYMOUS | MAP_PRIVATE |
                                MAP_FIXED_NOREPLACE,
                            -1, 0);
        if (mapped == MAP_FAILED) continue;
        if ((uintptr_t)mapped == candidates[index]) return mapped;
        /* Linux before 4.17 may ignore MAP_FIXED_NOREPLACE.  Never accept a
         * different address; unmap it and use the portable fallback below. */
        munmap(mapped, length);
      }
    }
  }
#endif

  /* Portable fallback. mmap() may ignore every hint, so validate the result. */
  static const int64_t offsets[TRAMP_NEAR_ATTEMPTS] = {
      0,  16 * 1024 * 1024LL,  -16 * 1024 * 1024LL,
      32 * 1024 * 1024LL, -32 * 1024 * 1024LL,
      48 * 1024 * 1024LL, -48 * 1024 * 1024LL,
      64 * 1024 * 1024LL, -64 * 1024 * 1024LL,
      80 * 1024 * 1024LL, -80 * 1024 * 1024LL,
      96 * 1024 * 1024LL, -96 * 1024 * 1024LL,
      112 * 1024 * 1024LL, -112 * 1024 * 1024LL,
      127 * 1024 * 1024LL};
  for (int i = 0; i < TRAMP_NEAR_ATTEMPTS; ++i) {
    uintptr_t hint_addr;
    if (offsets[i] < 0) {
      const uint64_t magnitude = (uint64_t)(-offsets[i]);
      if ((uint64_t)target < magnitude) continue;
      hint_addr = target - (uintptr_t)magnitude;
    } else {
      const uint64_t sum = (uint64_t)target + (uint64_t)offsets[i];
      if (sum < (uint64_t)target) continue;
      hint_addr = (uintptr_t)sum;
    }
    hint_addr &= ~(uintptr_t)(TRAMP_PAGE_SIZE - 1);
    void *mapped = mmap((void *)hint_addr, length, PROT_READ | PROT_WRITE,
                        MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
    if (mapped == MAP_FAILED) continue;
    uint32_t ignored;
    if (encode_b_imm26(&ignored, target,
                       (uintptr_t)mapped + 36) == 0) return mapped;
    munmap(mapped, length);
  }
  return MAP_FAILED;
}

static int install_impl(struct tramp *t, void *target, void *handler, int allow_adr) {
  if (!t || !target || !handler) {
    errno = EINVAL;
    return -1;
  }

  memcpy(t->saved_prologue, target, 20);

  uint32_t relocated[20];
  int words = relocate_prologue(relocated, t->saved_prologue, (uintptr_t)target, allow_adr);
  if (words < 0) {
    errno = ENOTSUP;
    return -2;
  }

  /* Allocate exec_buffer R/W only first; some kernels block PROT_EXEC on
   * fresh anonymous mappings (W^X policy). We mprotect to R/X after writing. */
  t->exec_buffer = mmap(NULL, TRAMP_PAGE_SIZE,
                        PROT_READ | PROT_WRITE,
                        MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
  if (t->exec_buffer == MAP_FAILED) {
    t->exec_buffer = NULL;
    return -3;
  }

  memcpy(t->exec_buffer, relocated, (size_t)words * 4);
  uint32_t branch_back[5];
  emit_long_branch(branch_back, (uintptr_t)target + 20);
  memcpy((uint8_t *)t->exec_buffer + words * 4, branch_back, 20);

  /* Now flip exec_buffer to R/X. */
  if (mprotect(t->exec_buffer, TRAMP_PAGE_SIZE,
               PROT_READ | PROT_EXEC) != 0) {
    int saved = errno;
    munmap(t->exec_buffer, TRAMP_PAGE_SIZE);
    t->exec_buffer = NULL;
    errno = saved;
    return -4;
  }

  uintptr_t page = (uintptr_t)target & ~(TRAMP_PAGE_SIZE - 1);
  size_t span = TRAMP_PAGE_SIZE;
  if (((uintptr_t)target + 20) > (page + TRAMP_PAGE_SIZE)) {
    span = 2 * TRAMP_PAGE_SIZE;
  }
  if (mprotect((void *)page, span, PROT_READ | PROT_WRITE | PROT_EXEC) != 0) {
    int saved = errno;
    munmap(t->exec_buffer, TRAMP_PAGE_SIZE);
    t->exec_buffer = NULL;
    errno = saved;
    return -4;
  }

  uint32_t to_handler[5];
  emit_long_branch(to_handler, (uintptr_t)handler);
  memcpy(target, to_handler, 20);

  if (mprotect((void *)page, span, PROT_READ | PROT_EXEC) != 0) {
    /* The patch is in place; we just couldn't restore page protection.
     * Continue — patch is functional. */
  }

  __builtin___clear_cache((char *)target, (char *)target + 20);
  __builtin___clear_cache((char *)t->exec_buffer,
                          (char *)t->exec_buffer + words * 4 + 20);

  t->target_addr = target;
  t->handler_addr = handler;
  return 0;
}

int tramp_install(struct tramp *t, void *target, void *handler) {
  return install_impl(t, target, handler, 0);
}

int tramp_install_with_adr(struct tramp *t, void *target, void *handler) {
  return install_impl(t, target, handler, 1);
}

int tramp_install_atomic(struct tramp *t, void *target, void *handler) {
  if (!t || !target || !handler || ((uintptr_t)target & 3u) != 0) {
    errno = EINVAL;
    return -1;
  }

  uint32_t first;
  memcpy(&first, target, sizeof first);
  if (!atomic_first_supported(first)) {
    errno = ENOTSUP;
    return -2;
  }

  /* The continuation is deliberately only one instruction long.  The old
   * five-instruction saved_prologue field remains populated at its head for
   * diagnostics, while no bytes beyond the first instruction are required. */
  memset(t->saved_prologue, 0, sizeof t->saved_prologue);
  memcpy(t->saved_prologue, &first, sizeof first);
  uint32_t relocated[20];
  const int words = relocate_prologue_count(
      relocated, t->saved_prologue, (uintptr_t)target, 1, 1);
  if (words < 0) {
    errno = ENOTSUP;
    return -2;
  }

  const size_t branch_back_offset = (size_t)words * sizeof(uint32_t);
  const size_t proxy_offset = branch_back_offset + 5 * sizeof(uint32_t);
  uint32_t to_proxy;
  void *exec = mmap_near_target((uintptr_t)target, TRAMP_PAGE_SIZE);
  if (exec == MAP_FAILED) {
    errno = ERANGE;
    return -3;
  }
  if (encode_b_imm26(&to_proxy, (uintptr_t)target,
                     (uintptr_t)exec + proxy_offset) != 0) {
    munmap(exec, TRAMP_PAGE_SIZE);
    errno = ERANGE;
    return -3;
  }

  memcpy(exec, relocated, (size_t)words * sizeof(uint32_t));
  uint32_t branch_back[5];
  emit_long_branch(branch_back, (uintptr_t)target + 4);
  memcpy((uint8_t *)exec + branch_back_offset, branch_back,
         sizeof branch_back);
  uint32_t to_handler[5];
  emit_long_branch(to_handler, (uintptr_t)handler);
  memcpy((uint8_t *)exec + proxy_offset, to_handler, sizeof to_handler);

  /* The proxy and continuation are immutable before the target is made
   * writable.  Flush the complete mapping after its final RX transition. */
  if (mprotect(exec, TRAMP_PAGE_SIZE, PROT_READ | PROT_EXEC) != 0) {
    const int saved_errno = errno;
    munmap(exec, TRAMP_PAGE_SIZE);
    errno = saved_errno;
    return -4;
  }
  __builtin___clear_cache((char *)exec,
                          (char *)exec + TRAMP_PAGE_SIZE);

  const uintptr_t page = (uintptr_t)target & ~(TRAMP_PAGE_SIZE - 1);
  size_t span = TRAMP_PAGE_SIZE;
  if ((uintptr_t)target + sizeof(uint32_t) > page + TRAMP_PAGE_SIZE)
    span = 2 * TRAMP_PAGE_SIZE;
  if (mprotect((void *)page, span, PROT_READ | PROT_WRITE | PROT_EXEC) != 0) {
    const int saved_errno = errno;
    munmap(exec, TRAMP_PAGE_SIZE);
    errno = saved_errno;
    return -4;
  }

  /* A callback may enter immediately after publication. Its original-call
   * continuation must be visible BEFORE the branch can become executable. */
  t->target_addr = target;
  t->handler_addr = handler;
  t->exec_buffer = exec;
  __atomic_thread_fence(__ATOMIC_RELEASE);

  /* This is the only text write in the atomic NAM path. */
  __atomic_store_n((uint32_t *)target, to_proxy, __ATOMIC_RELEASE);
  __builtin___clear_cache((char *)target, (char *)target + sizeof(uint32_t));

  if (mprotect((void *)page, span, PROT_READ | PROT_EXEC) != 0) {
    /* The hook and its RX mapping remain valid, but the target page is
     * writable.  It cannot be unmapped while the live hook may branch to it. */
    return TRAMP_INSTALL_ATOMIC_DEGRADED;
  }
  return 0;
}

int tramp_install_with_retry(struct tramp *t, void *target, void *handler,
                             int max_words, int *landed_offset_words) {
  int last_rc = -1;
  for (int w = 0; w <= max_words; w++) {
    void *try_target = (void *)((uintptr_t)target + (uintptr_t)(w * 4));
    int rc = tramp_install(t, try_target, handler);
    if (rc == 0) {
      if (landed_offset_words) *landed_offset_words = w;
      return 0;
    }
    last_rc = rc;
    /* Only retry on ENOTSUP (PC-relative op). Other errors (mmap, mprotect)
     * won't change with offset. */
    if (rc != -2) break;
  }
  if (landed_offset_words) *landed_offset_words = -1;
  return last_rc;
}
