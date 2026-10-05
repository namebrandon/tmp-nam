import { describe, expect, it } from "vitest";
import {
  activity,
  applyEvent,
  installStatus,
  itemDetail,
  nameList,
  percent,
  startOp,
  type OpItem,
  type Operation,
} from "../state/operation";

function item(op: Operation, i: number): OpItem {
  const it = op.items[i];
  if (!it) throw new Error(`no item ${String(i)}`);
  return it;
}

const files = [
  { name: "A.nam", label: "A", total: 1000 },
  { name: "B.nam", label: "B", total: 3000 },
];

describe("operation reducer", () => {
  it("folds send events into per-file rows and overall progress", () => {
    let op = startOp("send", files);
    expect(op.phase).toBe("send");
    op = applyEvent(op, { phase: "send", index: 0, done: 500, total: 1000 });
    expect(op.items[0]?.state).toBe("sending");
    expect(itemDetail(item(op, 0))).toBe("500 B of 1 KB");
    op = applyEvent(op, { phase: "sent", index: 0 });
    expect(op.items[0]?.state).toBe("sent");
    expect(percent(op)).toBe(25);
    expect(activity(op)).toEqual({
      title: "Sending captures",
      detail: "2 of 2 · B",
      value: 25,
    });
    op = applyEvent(op, { phase: "restart" });
    expect(percent(op)).toBeUndefined();
    expect(activity(op).title).toBe("Restarting audio engine");
  });

  it("counts downloads by item for installs", () => {
    let op = startOp("install", files);
    expect(op.phase).toBe("download");
    op = applyEvent(op, { phase: "download", index: 0, done: 1, total: 1 });
    expect(percent(op)).toBe(50);
    expect(installStatus(op, item(op, 0))).toBe("Waiting to send");
    expect(installStatus(op, item(op, 1))).toBe("Downloading");
    expect(activity(op)).toEqual({
      title: "Installing 2 captures",
      detail: "Downloading from Tone3000",
      value: 50,
    });
  });

  it("names files in a sentence", () => {
    expect(nameList(["A.nam"])).toBe("A");
    expect(nameList(["A.nam", "B.nam", "C.nam"])).toBe("A, B and C");
  });
});
