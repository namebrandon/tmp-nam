// Flows against the in-page mock backend (src/lib/mock.ts), one per page.
import { render, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import App from "../App";

const wait = { timeout: 4000 };

describe("Captures", () => {
  it("lists the unit's captures with the A2 size picker", async () => {
    render(<App />);
    expect(
      await screen.findByRole(
        "heading",
        { name: "Fender Deluxe Reverb '65 Vibrato" },
        wait,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Captures", { selector: "h1" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/4 on unit/)).toBeInTheDocument();
    const sizes = screen.getByRole("radiogroup", {
      name: "Size the player loads",
    });
    expect(within(sizes).getByRole("radio", { name: /Full/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByText("Connected · NAM card")).toBeInTheDocument();
  });

  it("notes that a size change applies next time", async () => {
    render(<App />);
    await screen.findByRole(
      "heading",
      { name: "Fender Deluxe Reverb '65 Vibrato" },
      wait,
    );
    expect(screen.queryByText("Output gain")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: /Lite/ }));
    expect(await screen.findByText("Saved to the unit")).toBeInTheDocument();
  });

  it("changes size without resending gain and explicitly clears Full", async () => {
    const save = vi.spyOn(api, "unitSetOptions");
    try {
      render(<App />);
      await screen.findByRole(
        "heading",
        { name: "Fender Deluxe Reverb '65 Vibrato" },
        wait,
      );
      const before = (await api.unitList()).models[0];
      if (!before) throw new Error("missing mock capture");
      expect(before.options.output_gain).toBeDefined();
      await userEvent.click(screen.getByRole("radio", { name: /Lite/ }));
      await screen.findByText("Saved to the unit");
      expect(save).toHaveBeenLastCalledWith(before.sha256, { size: 0 });
      expect((await api.unitList()).models[0]?.options.output_gain).toBe(
        before.options.output_gain,
      );
      await userEvent.click(screen.getByRole("radio", { name: /Full/ }));
      await waitFor(() => {
        expect(save).toHaveBeenLastCalledWith(before.sha256, { size: null });
      });
      await waitFor(async () => {
        const after = (await api.unitList()).models[0];
        if (!after) throw new Error("missing mock capture");
        expect(after.options.size).toBeUndefined();
        expect(after.options.output_gain).toBe(before.options.output_gain);
      });
    } finally {
      save.mockRestore();
    }
  });

  it("warns about unreliable settings until a successful refresh", async () => {
    const original = await api.unitList();
    const list = vi.spyOn(api, "unitList").mockResolvedValue({
      models: original.models.map((m) => ({ ...m, options: {} })),
      settings_error: "Cannot read /data/nam/player.json.",
    });
    try {
      render(<App />);
      expect(
        await screen.findByText("Player settings unavailable", {}, wait),
      ).toBeInTheDocument();
      expect(screen.getByText(/Current size is unknown/)).toBeInTheDocument();
      const sizes = screen.getAllByRole("radio");
      expect(sizes.length).toBeGreaterThan(0);
      for (const size of sizes)
        expect(size).toHaveAttribute("aria-checked", "false");
      list.mockResolvedValue(original);
      await userEvent.click(
        screen.getByRole("button", { name: "Refresh settings" }),
      );
      await waitFor(() =>
        expect(
          screen.queryByText("Player settings unavailable"),
        ).not.toBeInTheDocument(),
      );
      expect(
        screen
          .getAllByRole("radio")
          .some((size) => size.getAttribute("aria-checked") === "true"),
      ).toBe(true);
    } finally {
      list.mockRestore();
    }
  });

  it("shows settings errors even when no captures are installed", async () => {
    const list = vi.spyOn(api, "unitList").mockResolvedValue({
      models: [],
      settings_error: "Invalid player settings in /data/nam/player.json.",
    });
    try {
      render(<App />);
      expect(
        await screen.findByText("Player settings unavailable", {}, wait),
      ).toBeInTheDocument();
      expect(screen.getByText("No captures on the unit")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Refresh settings" }),
      ).toBeEnabled();
    } finally {
      list.mockRestore();
    }
  });

  it("shows the recovery warning and backup location after saving options", async () => {
    const warning =
      "Invalid player settings were reset. Backup: /data/nam/player.json.invalid.example. Reselect the capture on the unit.";
    const save = vi.spyOn(api, "unitSetOptions").mockResolvedValue(warning);
    try {
      render(<App />);
      await screen.findByRole(
        "heading",
        { name: "Fender Deluxe Reverb '65 Vibrato" },
        wait,
      );
      await userEvent.click(screen.getByRole("radio", { name: /Full/ }));
      expect(
        await screen.findByText("Player settings recovered"),
      ).toBeInTheDocument();
      expect(screen.getByText(warning)).toBeInTheDocument();
      expect(screen.getByText("Saved to the unit")).toBeInTheDocument();
    } finally {
      save.mockRestore();
    }
  });

  it("does not report a save or recovery when settings remain unreadable", async () => {
    const save = vi
      .spyOn(api, "unitSetOptions")
      .mockRejectedValue(
        new Error("cannot read player.json; settings were not changed"),
      );
    try {
      render(<App />);
      await screen.findByRole(
        "heading",
        { name: "Fender Deluxe Reverb '65 Vibrato" },
        wait,
      );
      await userEvent.click(screen.getByRole("radio", { name: /Full/ }));
      expect(
        await screen.findByText(/cannot read player.json/),
      ).toBeInTheDocument();
      expect(
        screen.queryByText("Player settings recovered"),
      ).not.toBeInTheDocument();
      expect(screen.queryByText("Saved to the unit")).not.toBeInTheDocument();
    } finally {
      save.mockRestore();
    }
  });

  it("checks files, sends the valid ones and reports the result", async () => {
    render(<App />);
    await screen.findByRole(
      "heading",
      { name: "Fender Deluxe Reverb '65 Vibrato" },
      wait,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Add Captures…" }),
    );
    const sheet = await screen.findByRole("dialog", {
      name: "Add captures to the unit",
    });
    expect(
      await within(sheet).findByText(/4 files checked\. 3 are valid captures/),
    ).toBeInTheDocument();
    expect(within(sheet).getByText("Skipped")).toBeInTheDocument();
    await userEvent.click(
      within(sheet).getByRole("button", { name: "Send 3 Captures" }),
    );
    expect(
      await screen.findByText("3 captures added", {}, wait),
    ).toBeInTheDocument();
    expect(screen.getAllByText("New")).toHaveLength(3);
    // Sent over the HID channel: no engine restart, so NAM keeps playing.
    expect(
      screen.queryByText("Reselect your preset on the unit"),
    ).not.toBeInTheDocument();
  });

  it("asks for a reselect when the send needed an engine restart", async () => {
    window.history.replaceState(null, "", "/?restart=1");
    try {
      render(<App />);
      await screen.findByRole(
        "heading",
        { name: "Fender Deluxe Reverb '65 Vibrato" },
        wait,
      );
      await userEvent.click(
        screen.getByRole("button", { name: "Add Captures…" }),
      );
      const sheet = await screen.findByRole("dialog", {
        name: "Add captures to the unit",
      });
      await userEvent.click(
        await within(sheet).findByRole("button", { name: "Send 3 Captures" }),
      );
      expect(
        await screen.findByText("Reselect your preset on the unit", {}, wait),
      ).toBeInTheDocument();
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });
});

describe("Tone3000", () => {
  it("asks for an API key before signing in", async () => {
    render(<App />);
    await userEvent.click(screen.getByRole("button", { name: "Tone3000" }));
    expect(
      await screen.findByText("Install tones from your Tone3000 account"),
    ).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("API key"), "t3k_pub_abc");
    await userEvent.click(screen.getByRole("button", { name: "Save Key" }));
    expect(
      await screen.findByRole("button", { name: "Sign In with Browser" }),
    ).toBeInTheDocument();
  });
});

describe("Tone3000 captures", () => {
  it("installs selected captures of a tone with several", async () => {
    render(<App />);
    await screen.findByRole(
      "heading",
      { name: "Fender Deluxe Reverb '65 Vibrato" },
      wait,
    );
    await userEvent.click(screen.getByRole("button", { name: "Tone3000" }));
    const key = await screen
      .findByLabelText("API key", {}, wait)
      .catch(() => null);
    if (key) {
      await userEvent.type(key, "t3k_pub_abc");
      await userEvent.click(screen.getByRole("button", { name: "Save Key" }));
    }
    await userEvent.click(
      await screen.findByRole("button", { name: "Sign In with Browser" }, wait),
    );
    await userEvent.click(
      await screen.findByRole(
        "button",
        { name: "Show captures of Boss SD-1 Super Overdrive" },
        wait,
      ),
    );
    await userEvent.click(screen.getByLabelText("Select APP-SD1-Drive-I"));
    await userEvent.click(screen.getByLabelText("Select APP-SD1-Boost-I"));
    expect(screen.getByText("2 captures from 1 tone")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Select Boss SD-1 Super Overdrive"),
    ).toHaveAttribute("aria-checked", "mixed");
    await userEvent.click(
      screen.getByRole("button", { name: "Install 2 on Unit" }),
    );
    expect(
      await screen.findByText("2 captures installed", {}, wait),
    ).toBeInTheDocument();
  });
});

describe("SD Card", () => {
  it("verifies the firmware and lists cards with refusals", async () => {
    render(<App />);
    await userEvent.click(screen.getByRole("button", { name: "SD Card" }));
    expect(
      await screen.findByText("Not a card reader", {}, wait),
    ).toBeInTheDocument();
    const create = screen.getByRole("button", { name: "Create SD Card…" });
    expect(create).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Choose File…" }));
    expect(await screen.findByText("Verified")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("radio", { name: /Generic STORAGE DEVICE/ }),
    );
    expect(create).toBeEnabled();
    await userEvent.click(create);
    expect(
      await screen.findByRole("alertdialog", {
        name: "Erase this card and create the NAM card?",
      }),
    ).toBeInTheDocument();
  });
});

describe("Settings", () => {
  it("offers diagnostics instead of raw unit details", async () => {
    render(<App />);
    await screen.findByRole(
      "heading",
      { name: "Fender Deluxe Reverb '65 Vibrato" },
      wait,
    );
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("tab", { name: "Unit" }));
    expect(screen.queryByText("nam-card-2026.10-r1")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy Diagnostics" }),
    ).toBeInTheDocument();
  });
});
