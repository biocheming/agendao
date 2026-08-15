import { describe, expect, it } from "vitest";
import {
  DIRECT_BLUEPRINT_STARTER,
  formatBlueprintDocument,
  parseBlueprintDocument,
} from "./blueprint";

describe("SchedulerBlueprint document", () => {
  it("round-trips the canonical v1 starter", () => {
    expect(parseBlueprintDocument(formatBlueprintDocument(DIRECT_BLUEPRINT_STARTER))).toEqual(
      DIRECT_BLUEPRINT_STARTER,
    );
  });

  it("rejects JSON that is not a Blueprint v1 object", () => {
    expect(() => parseBlueprintDocument('{"schema":"legacy"}')).toThrow(
      "Invalid SchedulerBlueprint v1 document",
    );
    expect(() => parseBlueprintDocument("[]")).toThrow(
      "Invalid SchedulerBlueprint v1 document",
    );
  });
});
