#include <cassert>
#include <filesystem>
#include <fstream>
#include <functional>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <thread>
#include <vector>

#include "NAM/container.h"
#include "NAM/dsp.h"
#include "NAM/get_dsp.h"
#include "NAM/registry.h"

namespace
{
class CountingDSP : public nam::DSP
{
public:
  explicit CountingDSP(const NAM_SAMPLE processValue = (NAM_SAMPLE)1.0)
  : nam::DSP(1, 1, 48000.0)
  , process_value(processValue)
  {
  }

  void Reset(const double sampleRate, const int maxBufferSize) override
  {
    if (on_reset)
      on_reset();
    ++reset_calls;
    reset_sample_rate = sampleRate;
    reset_buffer_size = maxBufferSize;
    nam::DSP::Reset(sampleRate, maxBufferSize);
  }

  void prewarm() override { ++prewarm_calls; }

  void process(NAM_SAMPLE** input, NAM_SAMPLE** output, const int numFrames) override
  {
    (void)input;
    ++process_calls;
    for (int i = 0; i < numFrames; ++i)
      output[0][i] = process_value;
  }

  const NAM_SAMPLE process_value;
  int reset_calls = 0;
  int prewarm_calls = 0;
  int process_calls = 0;
  double reset_sample_rate = 0.0;
  int reset_buffer_size = 0;
  std::function<void()> on_reset;
};

class ThrowingResetDSP final : public CountingDSP
{
public:
  void Reset(const double sampleRate, const int maxBufferSize) override
  {
    (void)sampleRate;
    (void)maxBufferSize;
    throw std::runtime_error("reset failure");
  }
};

NAM_SAMPLE ProcessOneSample(nam::DSP& dsp)
{
  NAM_SAMPLE input = (NAM_SAMPLE)0.0;
  NAM_SAMPLE output = (NAM_SAMPLE)-1.0;
  NAM_SAMPLE* inputPtr = &input;
  NAM_SAMPLE* outputPtr = &output;
  dsp.process(&inputPtr, &outputPtr, 1);
  return output;
}

std::unique_ptr<nam::container::ContainerModel> BuildContainer(CountingDSP*& small, CountingDSP*& large)
{
  auto smallModel = std::make_unique<CountingDSP>((NAM_SAMPLE)1.0);
  auto largeModel = std::make_unique<CountingDSP>((NAM_SAMPLE)2.0);
  small = smallModel.get();
  large = largeModel.get();
  std::vector<nam::container::Submodel> submodels;
  submodels.push_back({0.5, std::move(smallModel)});
  submodels.push_back({1.0, std::move(largeModel)});
  return std::make_unique<nam::container::ContainerModel>(std::move(submodels), 48000.0);
}

constexpr const char* kConstructorResetArchitecture = "TmpResetContractConstructor";
int gConstructCount = 0;
bool gThrowAfterConstruction = false;
bool gConstructedPolicy = true;
int gConstructedPrewarmCalls = -1;

std::unique_ptr<nam::DSP> ConstructorResetFactory(const nlohmann::json&, std::vector<float>&,
                                                  const double expectedSampleRate)
{
  auto dsp = std::make_unique<CountingDSP>();
  ++gConstructCount;
  dsp->Reset(expectedSampleRate, 16);
  gConstructedPolicy = dsp->GetPrewarmOnReset();
  gConstructedPrewarmCalls = dsp->prewarm_calls;
  if (gThrowAfterConstruction)
    throw std::runtime_error("constructor test failure");
  return dsp;
}

nam::factory::Helper gRegisterConstructorReset(kConstructorResetArchitecture, ConstructorResetFactory);

nlohmann::json BuildConstructorResetConfig()
{
  return {{"version", "0.7.0"},
          {"metadata", nlohmann::json::object()},
          {"architecture", kConstructorResetArchitecture},
          {"config", nlohmann::json::object()},
          {"weights", nlohmann::json::array()},
          {"sample_rate", 48000}};
}

CountingDSP* AsCountingDSP(const std::unique_ptr<nam::DSP>& dsp)
{
  auto* typed = dynamic_cast<CountingDSP*>(dsp.get());
  assert(typed != nullptr);
  return typed;
}

void TestStandaloneContract()
{
  CountingDSP defaultDsp;
  assert(defaultDsp.GetPrewarmOnReset());
  defaultDsp.Reset(44100.0, 32);
  assert(defaultDsp.reset_calls == 1);
  assert(defaultDsp.prewarm_calls == 1);
  assert(defaultDsp.reset_sample_rate == 44100.0);
  assert(defaultDsp.reset_buffer_size == 32);

  CountingDSP disabledDsp;
  disabledDsp.SetPrewarmOnReset(false);
  disabledDsp.Reset(48000.0, 64);
  assert(disabledDsp.prewarm_calls == 0);
  disabledDsp.ResetAndPrewarm(48000.0, 64);
  assert(disabledDsp.prewarm_calls == 1);
  assert(!disabledDsp.GetPrewarmOnReset());

  ThrowingResetDSP throwingDsp;
  throwingDsp.SetPrewarmOnReset(false);
  bool threw = false;
  try
  {
    throwingDsp.ResetAndPrewarm(48000.0, 64);
  }
  catch (const std::runtime_error&)
  {
    threw = true;
  }
  assert(threw);
  assert(!throwingDsp.GetPrewarmOnReset());
}

void TestThreadLocalDefaultContract()
{
  CountingDSP beforeScope;
  assert(beforeScope.GetPrewarmOnReset());
  {
    nam::ScopedPrewarmOnResetDefault disabled(false);
    CountingDSP inherited;
    assert(!inherited.GetPrewarmOnReset());

    bool otherThreadPolicy = false;
    std::thread otherThread([&]() {
      CountingDSP independent;
      otherThreadPolicy = independent.GetPrewarmOnReset();
    });
    otherThread.join();
    assert(otherThreadPolicy);

    {
      nam::ScopedPrewarmOnResetDefault enabled(true);
      CountingDSP nested;
      assert(nested.GetPrewarmOnReset());
    }
    CountingDSP restoredNested;
    assert(!restoredNested.GetPrewarmOnReset());
  }
  CountingDSP afterScope;
  assert(afterScope.GetPrewarmOnReset());

  try
  {
    nam::ScopedPrewarmOnResetDefault disabled(false);
    throw std::runtime_error("scope failure");
  }
  catch (const std::runtime_error&)
  {
  }
  CountingDSP afterThrow;
  assert(afterThrow.GetPrewarmOnReset());
}

void TestLoadOptionsContract()
{
  const auto config = BuildConstructorResetConfig();

  gConstructCount = 0;
  auto defaultDsp = nam::get_dsp(config);
  assert(gConstructCount == 1);
  assert(AsCountingDSP(defaultDsp)->prewarm_calls == 1);
  assert(defaultDsp->GetPrewarmOnReset());

  {
    nam::ScopedPrewarmOnResetDefault outer(false);
    auto inherited = nam::get_dsp(config);
    assert(AsCountingDSP(inherited)->prewarm_calls == 0);
    assert(!inherited->GetPrewarmOnReset());

    nam::DspLoadOptions forceWarm;
    forceWarm.prewarm = true;
    auto overridden = nam::get_dsp(config, forceWarm);
    assert(AsCountingDSP(overridden)->prewarm_calls == 1);
    assert(!overridden->GetPrewarmOnReset());

    CountingDSP restoredOuter;
    assert(!restoredOuter.GetPrewarmOnReset());
  }

  nam::DspLoadOptions suppressWarm;
  suppressWarm.prewarm = false;
  auto suppressed = nam::get_dsp(config, suppressWarm);
  assert(AsCountingDSP(suppressed)->prewarm_calls == 0);
  assert(suppressed->GetPrewarmOnReset());
  suppressed->Reset(48000.0, 16);
  assert(AsCountingDSP(suppressed)->prewarm_calls == 1);

  gThrowAfterConstruction = true;
  bool threw = false;
  try
  {
    (void)nam::get_dsp(config, suppressWarm);
  }
  catch (const std::runtime_error&)
  {
    threw = true;
  }
  gThrowAfterConstruction = false;
  assert(threw);
  assert(!gConstructedPolicy);
  assert(gConstructedPrewarmCalls == 0);
  CountingDSP afterFailedOverride;
  assert(afterFailedOverride.GetPrewarmOnReset());

  const auto path = std::filesystem::temp_directory_path() / "tmp-nam-reset-contract.nam";
  {
    std::ofstream output(path);
    output << config.dump();
  }
  nam::dspData returnedConfig;
  gConstructCount = 0;
  auto fromFile = nam::get_dsp(path, returnedConfig, suppressWarm);
  std::filesystem::remove(path);
  assert(fromFile != nullptr);
  assert(gConstructCount == 1);
  assert(returnedConfig.architecture == kConstructorResetArchitecture);
}

void TestContainerContract()
{
  {
    CountingDSP* small = nullptr;
    CountingDSP* large = nullptr;
    auto container = BuildContainer(small, large);
    container->prewarm();
    assert(small->prewarm_calls == 0);
    assert(large->prewarm_calls == 1);
  }

  {
    CountingDSP* small = nullptr;
    CountingDSP* large = nullptr;
    auto container = BuildContainer(small, large);
    container->Reset(44100.0, 128);
    assert(small->reset_calls == 0);
    assert(small->prewarm_calls == 0);
    assert(large->reset_calls == 1);
    assert(large->prewarm_calls == 1);
    assert(large->reset_sample_rate == 44100.0);
    assert(large->reset_buffer_size == 128);

    NAM_SAMPLE valueDuringReset = (NAM_SAMPLE)-1.0;
    small->on_reset = [&]() { valueDuringReset = ProcessOneSample(*container); };
    container->SetSlimmableSize(0.0);
    assert(small->reset_calls == 1);
    assert(small->prewarm_calls == 1);
    assert(valueDuringReset == large->process_value);
    assert(ProcessOneSample(*container) == small->process_value);
  }

  {
    CountingDSP* small = nullptr;
    CountingDSP* large = nullptr;
    auto container = BuildContainer(small, large);
    container->SetPrewarmOnReset(false);
    assert(!container->GetPrewarmOnReset());
    assert(!small->GetPrewarmOnReset());
    assert(!large->GetPrewarmOnReset());
    container->Reset(48000.0, 64);
    container->SetSlimmableSize(0.0);
    assert(small->prewarm_calls == 0);
    assert(large->prewarm_calls == 0);
    container->ResetAndPrewarm(48000.0, 64);
    assert(small->prewarm_calls == 1);
    assert(large->prewarm_calls == 0);
    assert(!container->GetPrewarmOnReset());
    assert(!small->GetPrewarmOnReset());
    assert(!large->GetPrewarmOnReset());
  }
}
} // namespace

int main()
{
  TestStandaloneContract();
  TestThreadLocalDefaultContract();
  TestLoadOptionsContract();
  TestContainerContract();
  std::cout << "NAM reset/prewarm contract: PASS\n";
}
