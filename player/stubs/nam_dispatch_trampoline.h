// Hand-rolled aarch64 long-branch trampoline runtime.
// Used by B.0 smoke tests (hitrate, loadFile-args) and the production
// nam_dispatch.so probe.
//
// Mechanism: at install time we (1) save the first 5 instructions of the
// target function, (2) overwrite them with a 5-instruction long-branch via
// x16 to our handler, (3) allocate an executable buffer containing the
// saved prologue followed by a long-branch back to (target + 20). Calling
// exec_buffer as a function executes the original prologue and resumes
// the rest of the original function.
//
// The installer aborts if any of the first 5 instructions is PC-relative
// (ADR, ADRP, B, BL, B.cond, CBZ, CBNZ, TBZ, TBNZ, LDR literal). Relocating
// those would need per-opcode rewriting, which we do not need for the
// recon-confirmed targets.

#ifndef NAM_DISPATCH_TRAMPOLINE_H
#define NAM_DISPATCH_TRAMPOLINE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct tramp {
  void *target_addr;
  void *handler_addr;
  void *exec_buffer; /* RX, 1 page; saved continuation at offset zero */
  uint8_t saved_prologue[20];
};

/* Positive result from tramp_install_atomic: the one-instruction hook is live,
 * but restoring the target page to RX failed, so the page remains writable. */
#define TRAMP_INSTALL_ATOMIC_DEGRADED 1

/* Install a long-branch trampoline at `target`, redirecting to `handler`.
 * Returns 0 on success, negative on failure. The negative value identifies
 * which step failed:
 *    -1 = invalid argument
 *    -2 = PC-relative instruction in saved prologue (errno=ENOTSUP)
 *    -3 = mmap failed (errno preserved)
 *    -4 = mprotect promote failed (errno preserved)
 *    -5 = mprotect restore failed (errno preserved)
 */
int tramp_install(struct tramp *t, void *target, void *handler);

/* Dtor-only variant: relocate ADR/ADRP by materializing their original absolute
 * value. Other PC-relative instructions are still rejected. Never skip a
 * prologue or change the original stack frame to find a patchable window. */
int tramp_install_with_adr(struct tramp *t, void *target, void *handler);

/* NAM-only live patch.  The target receives one aligned A64 B instruction to a
 * bounded nearby RX proxy.  The proxy long-branches to handler; exec_buffer at
 * offset zero contains the relocated first instruction and a long branch to
 * target+4 for the original continuation.  Returns 0 when the target page is
 * restored to RX, TRAMP_INSTALL_ATOMIC_DEGRADED when the hook is live but that
 * restore failed, or a negative error before any target text is changed. */
int tramp_install_atomic(struct tramp *t, void *target, void *handler);

/* Install a trampoline at `target`, retrying at `target+4`, `target+8`, ...
 * up to `target + max_words*4` if the initial install fails with ENOTSUP
 * (PC-relative op in the patched window). On success, `*landed_offset_words`
 * is set to the word offset where install actually succeeded (0 = original
 * target); the caller can use this to validate which prologue word was
 * trampolined. Useful for function entries where the very first instruction
 * is `bl <stack-check>` or `adrp x?, vtable` — those instructions are
 * still executed (the patched bytes start AFTER them) but we lose the
 * ability to capture them in the handler. Same return-value semantics as
 * tramp_install (most-recent-attempt's failure code).
 *
 * NOTE: when landed_offset_words > 0, the caller's handler still receives
 * x0..x7 with their values from the call site — the original first
 * instruction(s) executed normally before reaching the patched window.
 */
int tramp_install_with_retry(struct tramp *t, void *target, void *handler,
                             int max_words, int *landed_offset_words);

#ifdef __cplusplus
}
#endif

#endif
