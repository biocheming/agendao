import { describe, expect, it } from "vitest";
import { schedulerChoiceFromId } from "./webRuntime";

describe("schedulerChoiceFromId", () => {
  it("serializes auto using the backend tagged-union contract", () => {
    expect(schedulerChoiceFromId("auto")).toEqual({ kind: "auto" });
  });

  it("serializes built-in templates using canonical template IDs", () => {
    expect(schedulerChoiceFromId("verify")).toEqual({
      kind: "template",
      template: "verify",
    });
  });

  it("rejects scheduler IDs outside the canonical mode catalog", () => {
    expect(() => schedulerChoiceFromId("sisyphus")).toThrow("Unknown scheduler mode");
  });
});
