import { describe, expect, it } from "vitest";
import { reasoningLabel, supportedReasoningEfforts } from "./reasoning";

describe("reasoning effort picker", () => {
  it("keeps auto and off as distinct labels", () => {
    expect(reasoningLabel("")).toBe("Auto");
    expect(reasoningLabel("none")).toBe("Off");
  });

  it("uses capability metadata without confusing model variants for effort levels", () => {
    expect(
      supportedReasoningEfforts({
        id: "model",
        variants: ["fast", "thinking"],
        capabilities: { reasoning: true },
      }),
    ).toEqual(["minimal", "low", "medium", "high", "xhigh", "max", "ultra"]);
    expect(
      supportedReasoningEfforts({ id: "model", capabilities: { reasoning: false } }),
    ).toEqual([]);
  });
});
