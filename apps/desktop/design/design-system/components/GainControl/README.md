# GainControl

Sets a capture's output gain from 0× to 8× in 0.5 steps, with a stepper, a slider, its dB value and 1× / 4× presets.

- 1× means as captured; 0× is shown as "Muted". dB is 20·log10(×): 4× is +12.0 dB.
- The help line explains the 4× preset: NAM captures play about 12 dB quieter than the stock blocks. Hide it with `help={false}` where space is tight.
- `previous` shows "was 1×" until the change is applied on the unit.
- Pair with the `note` Banner: the new gain is heard the next time the capture is selected on the unit.
