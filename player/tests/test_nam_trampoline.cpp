// Pure instruction relocation tests: does NOT execute ARM code or patch text.
#include "../stubs/nam_dispatch_trampoline.c"
#include <cassert>
#include <iostream>

static uint32_t adr(unsigned rd, int displacement, bool page) {
  uint32_t imm = static_cast<uint32_t>(displacement) & 0x1fffff;
  return (page ? 0x90000000u : 0x10000000u) | ((imm & 3) << 29) |
         ((imm >> 2) << 5) | rd;
}
static uint64_t materialized(const uint32_t* instructions, unsigned rd) {
  uint64_t result = 0;
  for (int i = 0; i < 4; ++i) {
    assert((instructions[i] & 31) == rd);
    assert(((instructions[i] >> 21) & 3) == static_cast<unsigned>(i));
    assert((instructions[i] & 0xff800000u) == (i == 0 ? 0xd2800000u : 0xf2800000u));
    result |= uint64_t((instructions[i] >> 5) & 0xffff) << (16 * i);
  }
  return result;
}
int main() {
  const uintptr_t origin = 0x12345678;
  for (int displacement : {-1048576, -12345, -1, 0, 1, 12345, 1048575}) {
    for (bool page : {false, true}) {
      for (int index = 0; index < 5; ++index) {
        uint32_t saved[5] = {0xd503201f,0xd503201f,0xd503201f,0xd503201f,0xd503201f};
        saved[index] = adr(9, displacement, page);
        uint32_t out[20];
        assert(relocate_prologue(out, reinterpret_cast<uint8_t*>(saved), origin, 0) == -1);
        assert(relocate_prologue(out, reinterpret_cast<uint8_t*>(saved), origin, 1) == 8);
        uint64_t base = page ? ((origin + 4 * index) & ~uintptr_t(4095)) : origin + 4 * index;
        uint64_t expected = base + uint64_t(int64_t(displacement) * (page ? 4096 : 1));
        assert(materialized(out + index, 9) == expected);
      }
    }
  }
  for (uint32_t relative : {0x14000000u,0x94000000u,0x54000000u,0x34000000u,0x36000000u,0x58000000u}) {
    uint32_t saved[5] = {relative,0xd503201f,0xd503201f,0xd503201f,0xd503201f};
    uint32_t out[20];
    assert(relocate_prologue(out, reinterpret_cast<uint8_t*>(saved), origin, 1) == -1);
  }

  uint32_t branch = 0;
  const uintptr_t branch_pc = 0x10000000;
  assert(encode_b_imm26(&branch, branch_pc, branch_pc) == 0);
  assert(branch == 0x14000000u);
  assert(encode_b_imm26(&branch, branch_pc,
                        branch_pc + 0x07fffffcu) == 0);
  assert((branch & 0x03ffffffu) == 0x01ffffffu);
  assert(encode_b_imm26(&branch, branch_pc,
                        branch_pc - 0x08000000u) == 0);
  assert((branch & 0x03ffffffu) == 0x02000000u);
  assert(encode_b_imm26(&branch, branch_pc,
                        branch_pc + 0x08000000u) != 0);
  assert(encode_b_imm26(&branch, branch_pc,
                        branch_pc - 0x08000004u) != 0);
  assert(encode_b_imm26(&branch, branch_pc, branch_pc + 2) != 0);

  uint32_t first[5] = {0xd503201fu, 0, 0, 0, 0};  // nop
  uint32_t relocated_first[20] = {};
  assert(relocate_prologue_count(relocated_first,
                                 reinterpret_cast<uint8_t *>(first), origin,
                                 1, 1) == 1);
  assert(relocated_first[0] == first[0]);
  assert(atomic_first_supported(0xd503201fu));
  assert(atomic_first_supported(0x91000000u));  // add x0, x0, #0
  assert(!atomic_first_supported(0xd2800010u));  // movz x16, #0
  assert(!atomic_first_supported(adr(16, 0, false)));  // adr x16, #0
  assert(!atomic_first_supported(0x14000000u));  // b #0
  assert(!atomic_first_supported(0x58000000u));  // ldr x0, #0
  assert(!atomic_first_supported(0xd61f0000u));  // br x0
  assert(!atomic_first_supported(0xd65f03c0u));  // ret

  uint32_t rejected_target = 0x14000000u;
  struct tramp rejected_tramp = {};
  assert(tramp_install_atomic(&rejected_tramp, &rejected_target,
                               reinterpret_cast<void *>(origin)) == -2);
  assert(rejected_target == 0x14000000u);  // fail closed before text writes
  alignas(4) unsigned char unaligned_target[8] = {};
  assert(tramp_install_atomic(&rejected_tramp, unaligned_target + 1,
                               reinterpret_cast<void *>(origin)) == -1);

  uint32_t adr_first[5] = {adr(9, -12345, false), 0, 0, 0, 0};
  assert(relocate_prologue_count(relocated_first,
                                 reinterpret_cast<uint8_t *>(adr_first), origin,
                                 1, 1) == 4);
  assert(materialized(relocated_first, 9) == origin - 12345);

#if defined(__linux__)
  // Reproduce the 1.8.58 engine layout that defeated sparse mmap hints: the
  // image and heap cover every old probe, but a small valid branch-range gap
  // remains. MAP_FIXED_NOREPLACE must find that gap without replacing either
  // surrounding mapping.
  const uintptr_t synthetic_target = 0x50000000u;
  const size_t mib = 1024u * 1024u;
  void *lower = mmap(reinterpret_cast<void *>(synthetic_target - 127u * mib),
                     149u * mib, PROT_NONE,
                     MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED_NOREPLACE, -1, 0);
  assert(lower == reinterpret_cast<void *>(synthetic_target - 127u * mib));
  void *upper = mmap(reinterpret_cast<void *>(synthetic_target + 26u * mib),
                     101u * mib, PROT_NONE,
                     MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED_NOREPLACE, -1, 0);
  assert(upper == reinterpret_cast<void *>(synthetic_target + 26u * mib));
  void *near = mmap_near_target(synthetic_target, TRAMP_PAGE_SIZE);
  assert(near != MAP_FAILED);
  assert(reinterpret_cast<uintptr_t>(near) >= synthetic_target + 22u * mib);
  assert(reinterpret_cast<uintptr_t>(near) < synthetic_target + 26u * mib);
  munmap(near, TRAMP_PAGE_SIZE);
  munmap(upper, 101u * mib);
  munmap(lower, 149u * mib);
#endif

  std::cout << "NAM ADR/ADRP relocation and rejection: PASS\n";
}
